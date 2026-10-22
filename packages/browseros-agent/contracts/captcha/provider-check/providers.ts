import { readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'

const vendors = ['capsolver', 'capmonster'] as const
type Vendor = (typeof vendors)[number]
type Reply = Record<string, unknown>
type Options = {
  env?: Record<string, string | undefined>
  balanceOnly?: boolean
  requestTimeoutMs?: number
  pollIntervalMs?: number
  pollTimeoutMs?: number
}
type VendorResult = {
  vendor: Vendor
  status: 'ok' | 'error'
  balance: number | null
  task: {
    status: 'skipped' | 'ok' | 'error'
    elapsedMs?: number
    tiles?: number[]
    errorCode?: string
  }
  message: string
}

class ProviderError extends Error {
  constructor(
    readonly code: string,
    description: string,
  ) {
    super(`${code}: ${description}`)
  }
}

function invalid(description: string): never {
  throw new ProviderError('INVALID_RESPONSE', description)
}

async function post(
  url: string,
  method: string,
  body: Reply,
  timeoutMs: number,
): Promise<Reply> {
  const signal = AbortSignal.timeout(Math.max(1, Math.ceil(timeoutMs)))
  try {
    const response = await fetch(`${url.replace(/\/$/, '')}/${method}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      signal,
      redirect: 'error',
    })
    const reply: unknown = await response.json().catch((error: unknown) => {
      if (!response.ok)
        throw new ProviderError(`HTTP_${response.status}`, response.statusText)
      throw error
    })
    if (!reply || typeof reply !== 'object' || Array.isArray(reply))
      invalid('expected an object')
    const data = reply as Reply
    if (data.errorId !== 0) {
      throw new ProviderError(
        String(data.errorCode ?? 'VENDOR_ERROR'),
        String(data.errorDescription ?? 'missing errorId=0'),
      )
    }
    if (!response.ok)
      throw new ProviderError(`HTTP_${response.status}`, response.statusText)
    return data
  } catch (error) {
    if (signal.aborted)
      throw new ProviderError(
        'REQUEST_TIMEOUT',
        `${method} exceeded ${timeoutMs}ms`,
      )
    throw error
  }
}

function tiles(vendor: Vendor, reply: Reply): number[] {
  if (reply.status !== 'ready') invalid('task is not ready')
  const solution = reply.solution as Reply | undefined
  if (vendor === 'capsolver') {
    const indices = solution?.objects
    if (
      !Array.isArray(indices) ||
      !indices.every((n) => Number.isInteger(n) && n >= 0 && n < 9)
    )
      invalid('invalid tile indices')
    return indices
  }
  const answer = solution?.answer
  if (
    !Array.isArray(answer) ||
    answer.length !== 9 ||
    !answer.every((value) => typeof value === 'boolean')
  )
    invalid('invalid tile answers')
  return answer.flatMap((selected, index) => (selected ? [index] : []))
}

async function pollCapMonster(
  url: string,
  clientKey: string,
  taskId: unknown,
  timing: { requestMs: number; pollMs: number; pollTimeoutMs: number },
): Promise<Reply> {
  if (!Number.isInteger(taskId) || Number(taskId) <= 0)
    invalid('missing taskId')
  const deadline = performance.now() + timing.pollTimeoutMs
  while (true) {
    const remaining = deadline - performance.now()
    if (remaining > 0) await Bun.sleep(Math.min(timing.pollMs, remaining))
    const budget = deadline - performance.now()
    if (budget <= 0)
      throw new ProviderError('POLL_TIMEOUT', 'CapMonster did not become ready')
    const reply = await post(
      url,
      'getTaskResult',
      { clientKey, taskId },
      Math.min(timing.requestMs, budget),
    )
    if (reply.status === 'ready') return reply
    if (reply.status !== 'processing') invalid('unexpected task status')
  }
}

export async function loadKeys(env: Record<string, string | undefined>) {
  return Promise.all(
    vendors.map(async (vendor) => {
      const path =
        env[`BROWSEROS_${vendor.toUpperCase()}_API_KEY_FILE`] ??
        join(homedir(), '.config/browseros/captcha', `${vendor}-api-key`)
      try {
        const key = (await readFile(path, 'utf8')).trim()
        if (!key) throw new Error(`empty key file: ${path}`)
        return { vendor, key, error: '' }
      } catch (error) {
        return { vendor, key: '', error: `KEY_FILE: ${path}: ${String(error)}` }
      }
    }),
  )
}

export function redact(
  text: string,
  keys: Awaited<ReturnType<typeof loadKeys>>,
): string {
  const secrets = keys.flatMap(({ key }) =>
    key ? [key, JSON.stringify(key).slice(1, -1)] : [],
  )
  const unique = [...new Set(secrets)].sort((a, b) => b.length - a.length)
  if (!unique.length) return text
  const pattern = unique
    .map((value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('|')
  return text.replace(new RegExp(pattern, 'g'), '***')
}

export async function checkProviders(
  options: Options = {},
  loadedKeys?: Awaited<ReturnType<typeof loadKeys>>,
) {
  const env = options.env ?? process.env
  const keys = loadedKeys ?? (await loadKeys(env))
  const requestMs = options.requestTimeoutMs ?? 15_000
  const pollMs = options.pollIntervalMs ?? 3_000
  const pollTimeoutMs = options.pollTimeoutMs ?? 60_000
  let paidTasksCreated = 0
  const timestamp = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z')
  const results = await Promise.all(
    keys.map(async ({ vendor, key, error }): Promise<VendorResult> => {
      const result: VendorResult = {
        vendor,
        status: 'error',
        balance: null,
        task: { status: 'skipped' },
        message: '',
      }
      let started: number | undefined
      try {
        if (error) throw new Error(error)
        const url =
          env[`BROWSEROS_${vendor.toUpperCase()}_API_URL`] ??
          `https://api.${vendor === 'capsolver' ? 'capsolver.com' : 'capmonster.cloud'}`
        const balance = (
          await post(url, 'getBalance', { clientKey: key }, requestMs)
        ).balance
        if (
          typeof balance !== 'number' ||
          !Number.isFinite(balance) ||
          balance < 0
        )
          invalid('invalid balance')
        result.balance = balance
        result.status = 'ok'
        if (balance === 0 || options.balanceOnly) {
          result.message =
            balance === 0
              ? 'balance is 0; paid task not attempted'
              : '--balance-only; paid task not attempted'
          return result
        }
        const image = Buffer.from(
          await Bun.file(
            join(import.meta.dir, 'sample/grid.png'),
          ).arrayBuffer(),
        ).toString('base64')
        const task =
          vendor === 'capsolver'
            ? {
                type: 'ReCaptchaV2Classification',
                image,
                question: '/m/015qff',
              }
            : {
                type: 'ComplexImageTask',
                class: 'recaptcha',
                imagesBase64: [image],
                metadata: { Grid: '3x3', TaskDefinition: '/m/015qff' },
              }
        started = performance.now()
        result.task.status = 'error'
        paidTasksCreated++
        let reply = await post(
          url,
          'createTask',
          { clientKey: key, task },
          requestMs,
        )
        if (vendor === 'capmonster') {
          reply = await pollCapMonster(url, key, reply.taskId, {
            requestMs,
            pollMs,
            pollTimeoutMs,
          })
        }
        result.task = {
          status: 'ok',
          elapsedMs: Math.round(performance.now() - started),
          tiles: tiles(vendor, reply),
        }
      } catch (error) {
        result.status = 'error'
        result.message = String(error instanceof Error ? error.message : error)
        if (started !== undefined) {
          result.task = {
            status: 'error',
            elapsedMs: Math.round(performance.now() - started),
            errorCode:
              error instanceof ProviderError ? error.code : 'TASK_ERROR',
          }
        }
      }
      return result
    }),
  )
  const ready = (vendor: Vendor) =>
    results.some(
      (r) =>
        r.vendor === vendor &&
        r.balance !== null &&
        r.balance > 0 &&
        r.task.status === 'ok',
    )
  const report = {
    timestamp,
    vendors: results,
    paidTasksCreated,
    paidSolvingReady: {
      capsolver: ready('capsolver'),
      capmonster: ready('capmonster'),
    },
    exitCode: results.every((r) => r.status === 'ok' && r.task.status === 'ok')
      ? 0
      : 1,
  }
  return { report, redact: (text: string) => redact(text, keys) }
}
