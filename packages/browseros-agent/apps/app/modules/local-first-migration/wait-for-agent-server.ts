import { getAgentServerUrl } from '@/lib/browseros/helpers'

/**
 * Long enough to cover a cold start where the server is booting alongside the
 * browser and has migrations of its own to apply, short enough that a genuinely
 * absent server does not keep a background task alive all session.
 */
export const SERVER_WAIT_TIMEOUT_MS = 60_000
export const SERVER_WAIT_INTERVAL_MS = 1_000

export interface WaitForAgentServerOptions {
  isHealthy?: () => Promise<boolean>
  timeoutMs?: number
  intervalMs?: number
  now?: () => number
  sleep?: (ms: number) => Promise<void>
}

/**
 * Health on the agent server itself, not `getHealthCheckUrl`.
 *
 * That helper resolves the proxy port, while the imports address the agent
 * server on the mcp port. They are separate services that can become ready at
 * different moments, so probing the proxy would answer a question nobody
 * asked: a proxy up first would wave the imports through into the same race
 * this exists to close, and a proxy that is down would defer imports the agent
 * server was ready to accept.
 */
export async function agentServerHealthUrl(): Promise<string> {
  return `${await getAgentServerUrl()}/system/health`
}

async function defaultIsHealthy(): Promise<boolean> {
  try {
    const response = await fetch(await agentServerHealthUrl())
    return response.ok
  } catch {
    return false
  }
}

const defaultSleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms))

/**
 * Waits until the local server answers, or gives up.
 *
 * The one-time import runs when the background starts, which is the same moment
 * the server starts. It used to fire straight into a socket nothing was
 * listening on yet and fail, and because a failed run leaves its marker unset
 * it simply lost the same race on the next launch, so the imported data never
 * appeared at all.
 *
 * Polling health first is what makes the import wait for its dependency rather
 * than race it. Returning false rather than throwing keeps the caller's
 * decision explicit: leave the markers unset and try again next start.
 */
export async function waitForAgentServer({
  isHealthy = defaultIsHealthy,
  timeoutMs = SERVER_WAIT_TIMEOUT_MS,
  intervalMs = SERVER_WAIT_INTERVAL_MS,
  now = Date.now,
  sleep = defaultSleep,
}: WaitForAgentServerOptions = {}): Promise<boolean> {
  const deadline = now() + timeoutMs

  while (true) {
    // A probe that throws means not reachable, not a reason to abandon the
    // wait. The default one already swallows fetch errors; catching here means
    // any probe behaves the same way.
    let healthy = false
    try {
      healthy = await isHealthy()
    } catch {
      healthy = false
    }
    if (healthy) return true
    if (now() >= deadline) return false
    await sleep(intervalMs)
  }
}
