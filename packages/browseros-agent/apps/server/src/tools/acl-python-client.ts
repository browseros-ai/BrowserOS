import { fileURLToPath } from 'node:url'
import type { AclRule, ElementProperties } from '@browseros/shared/types/acl'
import { logger } from '../lib/logger'

const ACL_PYTHON_ENABLED_VALUES = new Set(['1', 'true', 'yes'])

interface PythonAclPayload {
  toolName: string
  pageUrl: string
  element: ElementProperties
  rules: AclRule[]
}

interface PythonAclResult {
  blocked: boolean
  matchedRuleId?: string | null
  confidence?: number
  semanticScore?: number
  semanticBackend?: string
}

interface PythonAclResponse {
  id: number
  ok: boolean
  result?: PythonAclResult
  error?: string
  traceback?: string
}

interface PendingRequest {
  resolve: (value: PythonAclResult) => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}

function isPythonAclEnabled(): boolean {
  const value = process.env.BROWSEROS_ACL_PYTHON?.trim().toLowerCase()
  return value ? ACL_PYTHON_ENABLED_VALUES.has(value) : false
}

function getPythonTimeoutMs(): number {
  const raw = process.env.BROWSEROS_ACL_PYTHON_TIMEOUT_MS
  if (!raw) return 30_000

  const parsed = Number(raw)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 30_000
}

function getPythonCommand(): string[] {
  if (process.env.BROWSEROS_ACL_PYTHON_BIN) {
    return [process.env.BROWSEROS_ACL_PYTHON_BIN, '-m', 'acl_lab.rpc']
  }
  return ['uv', 'run', 'python', '-m', 'acl_lab.rpc']
}

function getPythonWorkerCwd(): string {
  if (process.env.BROWSEROS_ACL_PYTHON_CWD) {
    return process.env.BROWSEROS_ACL_PYTHON_CWD
  }
  return fileURLToPath(new URL('../../../../python/acl_lab/', import.meta.url))
}

class AclPythonClient {
  private proc?: Bun.Subprocess<'pipe', 'pipe', 'pipe'>
  private nextId = 1
  private stdoutBuffer = ''
  private stderrBuffer = ''
  private pending = new Map<number, PendingRequest>()

  async checkAcl(payload: PythonAclPayload): Promise<PythonAclResult> {
    const proc = this.ensureProcess()
    const stdin = proc.stdin
    if (!stdin) {
      throw new Error('ACL Python worker stdin is unavailable')
    }

    const id = this.nextId++
    const body = JSON.stringify({ id, type: 'check_acl', payload }) + '\n'

    return new Promise<PythonAclResult>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(
          new Error(
            `ACL Python worker request timed out after ${getPythonTimeoutMs()}ms`,
          ),
        )
      }, getPythonTimeoutMs())

      this.pending.set(id, { resolve, reject, timer })

      try {
        stdin.write(body)
        stdin.flush()
      } catch (error) {
        clearTimeout(timer)
        this.pending.delete(id)
        reject(
          error instanceof Error
            ? error
            : new Error(
                `Failed to write to ACL Python worker: ${String(error)}`,
              ),
        )
      }
    })
  }

  private ensureProcess(): Bun.Subprocess<'pipe', 'pipe', 'pipe'> {
    if (this.proc && this.proc.exitCode === null) {
      return this.proc
    }

    this.proc = Bun.spawn(getPythonCommand(), {
      cwd: getPythonWorkerCwd(),
      stdin: 'pipe',
      stdout: 'pipe',
      stderr: 'pipe',
      env: { ...process.env, PYTHONUNBUFFERED: '1' },
    })

    void this.readStdout(this.proc)
    void this.readStderr(this.proc)
    void this.proc.exited.then((exitCode) => {
      if (exitCode !== 0) {
        logger.warn('ACL Python worker exited', { exitCode })
      }
      this.failAllPending(
        new Error(`ACL Python worker exited with code ${exitCode}`),
      )
      this.proc = undefined
    })

    return this.proc
  }

  private async readStdout(
    proc: Bun.Subprocess<'pipe', 'pipe', 'pipe'>,
  ): Promise<void> {
    const reader = proc.stdout.getReader()
    const decoder = new TextDecoder()

    try {
      while (true) {
        const { value, done } = await reader.read()
        if (done) break
        this.stdoutBuffer += decoder.decode(value, { stream: true })
        this.processStdoutLines()
      }
      this.stdoutBuffer += decoder.decode()
      this.processStdoutLines()
    } finally {
      reader.releaseLock()
    }
  }

  private async readStderr(
    proc: Bun.Subprocess<'pipe', 'pipe', 'pipe'>,
  ): Promise<void> {
    const reader = proc.stderr.getReader()
    const decoder = new TextDecoder()

    try {
      while (true) {
        const { value, done } = await reader.read()
        if (done) break
        this.stderrBuffer += decoder.decode(value, { stream: true })
        this.flushStderrLines()
      }
      this.stderrBuffer += decoder.decode()
      this.flushStderrLines(true)
    } finally {
      reader.releaseLock()
    }
  }

  private processStdoutLines(): void {
    const lines = this.stdoutBuffer.split('\n')
    this.stdoutBuffer = lines.pop() ?? ''

    for (const line of lines) {
      const trimmed = line.trim()
      if (!trimmed) continue

      let parsed: PythonAclResponse
      try {
        parsed = JSON.parse(trimmed) as PythonAclResponse
      } catch {
        logger.warn('ACL Python worker returned invalid JSON', {
          line: trimmed,
        })
        continue
      }

      const pending = this.pending.get(parsed.id)
      if (!pending) continue

      clearTimeout(pending.timer)
      this.pending.delete(parsed.id)

      if (!parsed.ok || !parsed.result) {
        pending.reject(
          new Error(
            parsed.error || 'ACL Python worker returned an empty response',
          ),
        )
        continue
      }

      pending.resolve(parsed.result)
    }
  }

  private flushStderrLines(flushTail = false): void {
    const lines = this.stderrBuffer.split('\n')
    this.stderrBuffer = flushTail ? '' : (lines.pop() ?? '')

    for (const line of flushTail ? lines.filter(Boolean) : lines) {
      const trimmed = line.trim()
      if (!trimmed) continue
      logger.warn('ACL Python worker stderr', { line: trimmed })
    }
  }

  private failAllPending(error: Error): void {
    for (const [id, pending] of this.pending.entries()) {
      clearTimeout(pending.timer)
      pending.reject(error)
      this.pending.delete(id)
    }
  }
}

let aclPythonClient: AclPythonClient | undefined

export async function checkAclWithPython(
  payload: PythonAclPayload,
): Promise<PythonAclResult | null> {
  if (!isPythonAclEnabled()) return null

  try {
    aclPythonClient ??= new AclPythonClient()
    return await aclPythonClient.checkAcl(payload)
  } catch (error) {
    logger.warn(
      'ACL Python matcher failed; allowing action without TS fallback',
      {
        error: error instanceof Error ? error.message : String(error),
      },
    )
    return { blocked: false }
  }
}
