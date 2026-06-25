import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

const screenshotCaptureQueues = new WeakMap<ProtocolApi, Promise<void>>()

/** Serializes captures because annotation overlay DOM is page-global while screenshots are in flight. */
export async function runExclusiveScreenshotCapture<T>(
  pageSession: ProtocolApi,
  task: () => Promise<T>,
): Promise<T> {
  const previous = screenshotCaptureQueues.get(pageSession) ?? Promise.resolve()
  let releaseCurrent = () => {}
  const current = new Promise<void>((resolve) => {
    releaseCurrent = resolve
  })
  const tail = previous.catch(() => {}).then(() => current)
  screenshotCaptureQueues.set(pageSession, tail)

  await previous.catch(() => {})
  try {
    return await task()
  } finally {
    releaseCurrent()
    if (screenshotCaptureQueues.get(pageSession) === tail) {
      screenshotCaptureQueues.delete(pageSession)
    }
  }
}
