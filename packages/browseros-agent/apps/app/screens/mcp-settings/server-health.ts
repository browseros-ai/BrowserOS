import { sendServerMessage } from '@/lib/messaging/server/serverMessages'

const HEALTH_CHECK_TIMEOUT_MS = 60_000
const HEALTH_CHECK_INTERVAL_MS = 2_000

async function checkServerHealth(): Promise<boolean> {
  try {
    const result = await sendServerMessage('checkHealth', undefined)
    return result.healthy
  } catch {
    return false
  }
}

/**
 * Poll the local health endpoint until the server responds or the timeout
 * elapses. checkHealth resolves its URL from the live proxy_port pref, so this
 * automatically targets whatever port the server is (re)binding to.
 */
export async function waitForServerHealth(): Promise<boolean> {
  const startTime = Date.now()
  return new Promise((resolve) => {
    const check = async () => {
      if (Date.now() - startTime >= HEALTH_CHECK_TIMEOUT_MS) {
        resolve(false)
        return
      }
      if (await checkServerHealth()) {
        resolve(true)
        return
      }
      setTimeout(check, HEALTH_CHECK_INTERVAL_MS)
    }
    setTimeout(check, HEALTH_CHECK_INTERVAL_MS)
  })
}
