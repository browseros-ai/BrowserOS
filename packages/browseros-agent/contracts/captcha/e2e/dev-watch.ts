import { execFile, spawn } from 'node:child_process'
import { mkdir, mkdtemp, open, readFile, realpath, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { promisify } from 'node:util'
import { McpSession } from '../../claw-mcp/tests/mcp-client'

const ownerPorts = new Set([9010, 9011, 9121, 9211, 9212, 9213, 9214, 9215])

export function parseWatchPorts(log: string) {
  const match = log.match(
    /Ports: CDP=(\d+) Server=(\d+) Extension=(\d+)(?:\s|$)/,
  )
  if (!match) return null
  const [cdp, server, extension] = [
    Number(match[1]),
    Number(match[2]),
    Number(match[3]),
  ]
  if ([cdp, server, extension].some((p) => !p || p > 65535)) return null
  return { cdp, server, extension }
}

export interface Preflight {
  url: string
  port: number
  hostedMode: '1' | '0'
  toolPresent: boolean | null
  health: unknown
  ok: boolean
  error?: string
}

function connected(health: unknown): boolean {
  const value = health as {
    status?: string
    browser?: { connected?: boolean }
  } | null
  return value?.status === 'ok' && value.browser?.connected === true
}

async function fetchHealth(url: string): Promise<unknown> {
  const response = await fetch(new URL('/system/health', url), {
    signal: AbortSignal.timeout(1000),
  })
  if (!response.ok) throw new Error(`health HTTP ${response.status}`)
  return response.json()
}

export async function checkDevPreflight(
  url: string,
  expectToolMissing = false,
  dependencies = {
    health: () => fetchHealth(url),
    tools: async (): Promise<readonly { name: string }[]> => {
      const session = await McpSession.connect(
        url.replace(/\/mcp\/?$/, ''),
        'captcha-preflight',
      )
      try {
        return await session.listTools()
      } finally {
        await session.close().catch(() => {})
      }
    },
  },
  hostedMode: '1' | '0' = '1',
): Promise<Preflight> {
  const result: Preflight = {
    url,
    port: Number(new URL(url).port),
    hostedMode,
    toolPresent: null,
    health: null,
    ok: false,
  }
  try {
    if (ownerPorts.has(result.port))
      throw new Error(`refusing owner port ${result.port}`)
    result.health = await dependencies.health()
    if (!connected(result.health))
      throw new Error('health must be ok with browser connected')
    result.toolPresent = (await dependencies.tools()).some(
      (tool) => tool.name === 'solve_captcha',
    )
    if (result.toolPresent === expectToolMissing)
      throw new Error(
        `solve_captcha is ${result.toolPresent ? 'present but --expect-tool-missing was set' : 'missing'}`,
      )
    result.ok = true
  } catch (error) {
    result.error =
      error instanceof Error ? error.message : 'preflight request failed'
  }
  return result
}

export function renderPreflight(preflight: Preflight): string {
  return `Preflight: ${JSON.stringify(preflight)}\n\n`
}

function spawnWatch(env: NodeJS.ProcessEnv, fd: number) {
  const child = spawn('bun', ['run', 'dev:claw:watch:new'], {
    cwd: resolve(import.meta.dir, '../../..'),
    detached: true,
    env,
    stdio: ['ignore', fd, fd],
  })
  const exited = new Promise<void>((resolve, reject) => {
    child.once('exit', () => resolve())
    child.once('error', reject)
  })
  return { pid: child.pid, exited }
}

const defaults = {
  spawn: spawnWatch,
  health: fetchHealth,
  graceMs: 15_000,
  supervisor: async (pid: number) => {
    try {
      const { stdout } = await promisify(execFile)('pgrep', ['-P', String(pid)])
      const children = stdout.trim().split(/\s+/).map(Number)
      return children.length === 1 ? (children[0] ?? pid) : pid
    } catch (error) {
      if ((error as { code?: number }).code !== 1) throw error
      return pid
    }
  },
  signal: (pid: number, signal: NodeJS.Signals) => {
    try {
      process.kill(pid, signal)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error
    }
  },
  processes: async () =>
    (
      await promisify(execFile)('ps', ['eww', '-axo', 'pid=,command='], {
        maxBuffer: 16 * 1024 * 1024,
      })
    ).stdout,
}

async function waitForExit(exited: Promise<unknown>, ms: number) {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    await Promise.race([
      exited,
      new Promise((resolve) => {
        timer = setTimeout(resolve, ms)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

/** Registers a handle immediately; waiting for readiness must happen after ownership. */
export async function startDevWatch(
  options: {
    out: string
    startupTimeout: number
    keyEnv: NodeJS.ProcessEnv
    hostedMode?: '1' | '0'
  },
  overrides: Partial<typeof defaults> = {},
) {
  const dependencies = { ...defaults, ...overrides }
  const directory = await mkdtemp(
    join(await realpath(tmpdir()), 'captcha-dev-watch-'),
  )
  const logPath = join(options.out, 'dev-watch.log')
  let child: ReturnType<typeof spawnWatch>
  try {
    await mkdir(options.out, { recursive: true })
    await mkdir(join(directory, 'state'))
    const log = await open(logPath, 'w')
    try {
      child = dependencies.spawn(
        {
          ...process.env,
          ...options.keyEnv,
          BROWSERCLAW_HOSTED_MODE: options.hostedMode ?? '1',
          BROWSERCLAW_DIR: join(directory, 'state'),
          TMPDIR: directory,
          CARGO_TARGET_DIR:
            // biome-ignore lint/suspicious/noUndeclaredEnvVars: this standalone test runner is not a Turbo task.
            process.env.CARGO_TARGET_DIR ||
            '/Volumes/extssd/cargo-target/captcha-e2e-dev-watch',
        },
        log.fd,
      )
    } finally {
      await log.close()
    }
  } catch (error) {
    await rm(directory, { recursive: true, force: true })
    throw error
  }
  let finished = false
  let stopped: Promise<void> | undefined
  const exited = child.exited
    .catch(() => {})
    .finally(() => {
      finished = true
    })
  const deadline = Date.now() + options.startupTimeout
  return {
    directory,
    logPath,
    async ready() {
      let waiting = 'Ports: CDP=<c> Server=<s> Extension=<e>'
      do {
        if (stopped) throw new Error('dev-watch interrupted')
        if (finished)
          throw new Error(`dev-watch exited before ready; see ${logPath}`)
        const ports = parseWatchPorts(await readFile(logPath, 'utf8'))
        if (ports) {
          for (const port of Object.values(ports))
            if (ownerPorts.has(port))
              throw new Error(`refusing owner port ${port}`)
          const url = `http://127.0.0.1:${ports.server}/mcp`
          waiting = 'health status ok and browser connected'
          try {
            if (connected(await dependencies.health(url))) return url
          } catch {
            /* The server may still be compiling. */
          }
        }
        await Bun.sleep(Math.min(250, Math.max(1, deadline - Date.now())))
      } while (Date.now() < deadline)
      throw new Error(
        `dev-watch startup timed out after ${options.startupTimeout} ms waiting for ${waiting}; see ${logPath}`,
      )
    },
    stop() {
      stopped ??= (async () => {
        if (child.pid && !finished) {
          const supervisor = await dependencies.supervisor(child.pid)
          // Bun forwards signals: hitting its whole group sends SIGINT twice.
          if (!finished) dependencies.signal(supervisor, 'SIGINT')
          await waitForExit(exited, dependencies.graceMs)
          if (!finished) {
            dependencies.signal(-child.pid, 'SIGKILL')
            await waitForExit(exited, 3000)
          }
        }
        const deadline = Date.now() + dependencies.graceMs
        let leaks: string[] = []
        while (true) {
          leaks = (await dependencies.processes())
            .split('\n')
            .filter((line) => line.includes(directory))
            .map((line) => line.trim().split(/\s+/)[0] ?? '')
          if (!leaks.length || Date.now() >= deadline) break
          await Bun.sleep(Math.min(100, Math.max(1, deadline - Date.now())))
        }
        if (leaks.length)
          throw new Error(
            `dev-watch cleanup failed: leftover PIDs ${leaks.join(', ')} reference ${directory}`,
          )
        await rm(directory, { recursive: true, force: true })
      })()
      return stopped
    },
  }
}
