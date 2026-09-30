import { getAgentServerUrl } from '@/lib/browseros/helpers'

const MAX_ATTEMPTS = 3
const RETRY_DELAY_MS = 500

type ResolveAgentServerUrlOptions = {
  read?: () => Promise<string>
  maxAttempts?: number
  retryDelayMs?: number
  sleep?: (ms: number) => Promise<void>
}

const defaultSleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms))

/**
 * The local server's base URL, waiting out the startup window.
 *
 * The URL is built from a browser pref the binary publishes shortly after the
 * extension loads, so a surface that asks too early gets a throw rather than a
 * URL. getAgentServerUrl is that single read; this waits for it. Anything that
 * can run before the browser has settled wants this one.
 */
export async function resolveAgentServerUrl({
  read = getAgentServerUrl,
  maxAttempts = MAX_ATTEMPTS,
  retryDelayMs = RETRY_DELAY_MS,
  sleep = defaultSleep,
}: ResolveAgentServerUrlOptions = {}): Promise<string> {
  let lastError: unknown

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    try {
      return await read()
    } catch (error) {
      lastError = error
      if (attempt < maxAttempts) {
        await sleep(retryDelayMs)
      }
    }
  }

  throw lastError instanceof Error ? lastError : new Error(String(lastError))
}
