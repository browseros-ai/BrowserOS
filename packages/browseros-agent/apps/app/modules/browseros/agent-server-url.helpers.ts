import { getAgentServerUrl } from '@/lib/browseros/helpers'

export const MAX_AGENT_SERVER_URL_ATTEMPTS = 3
export const AGENT_SERVER_URL_RETRY_DELAY_MS = 500

export type ResolveAgentServerUrlWithRetryOptions = {
  resolve?: () => Promise<string>
  maxAttempts?: number
  retryDelayMs?: number
  sleep?: (ms: number) => Promise<void>
}

const defaultSleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms))

export async function resolveAgentServerUrlWithRetry({
  resolve = getAgentServerUrl,
  maxAttempts = MAX_AGENT_SERVER_URL_ATTEMPTS,
  retryDelayMs = AGENT_SERVER_URL_RETRY_DELAY_MS,
  sleep = defaultSleep,
}: ResolveAgentServerUrlWithRetryOptions = {}): Promise<string> {
  let lastError: unknown

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    try {
      return await resolve()
    } catch (error) {
      lastError = error
      if (attempt < maxAttempts) {
        await sleep(retryDelayMs)
      }
    }
  }

  throw lastError instanceof Error ? lastError : new Error(String(lastError))
}
