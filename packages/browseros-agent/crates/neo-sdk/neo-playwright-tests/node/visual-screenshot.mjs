/** Playwright Test owns baselines and comparison; Neo owns all page reads and pixel capture. */
import { setTimeout as sleep } from 'node:timers/promises'

const pixelOptions = [
  'animations',
  'caret',
  'clip',
  'fullPage',
  'mask',
  'maskColor',
  'omitBackground',
  'scale',
  'style',
  'type',
]

/** Adapt only the runner's visual matcher seam, without exposing a reference browser object. */
export function screenshotReceiver(actual, server) {
  const locator = typeof actual.page === 'function' ? actual : undefined
  const page = locator ? locator.page() : actual
  const receiver = {
    _apiName: 'Page',
    // Playwright's screencast overlays are absent here. Neo's capture path already owns
    // hiding and restoring its own pointer, including on a failed or cancelled capture.
    screencast: { hideOverlays: async () => {}, showOverlays: async () => {} },
    _expectScreenshot: (options) => expectScreenshot(page, options, server),
  }
  return locator
    ? {
        _apiName: 'Locator',
        page: () => receiver,
        toString: () => locator.toString(),
        neo: locator,
      }
    : receiver
}

async function expectScreenshot(page, options, server) {
  const controller = new AbortController()
  const deadline = options.timeout
    ? performance.now() + options.timeout
    : Infinity
  let timedOut = false
  const cancel = () =>
    controller.abort(
      new Error(`Assertion aborted: ${String(options.signal.reason ?? '')}`),
    )
  let rejectAbort
  const aborted = new Promise((_, reject) => {
    rejectAbort = reject
  })
  aborted.catch(() => {})
  controller.signal.addEventListener(
    'abort',
    () => {
      rejectAbort(
        controller.signal.reason ?? new Error('Screenshot assertion aborted'),
      )
    },
    { once: true },
  )
  if (options.signal?.aborted) cancel()
  else options.signal?.addEventListener('abort', cancel, { once: true })
  const timer = options.timeout
    ? setTimeout(() => {
        timedOut = true
        controller.abort(new Error(`Timeout ${options.timeout}ms exceeded.`))
      }, options.timeout)
    : undefined
  const log = []
  const progress = {
    ...server.nullProgress,
    metadata: { ...server.nullProgress.metadata, log },
    signal: controller.signal,
    log: (message) => log.push(message),
    race: (promise) =>
      Promise.race([
        ...(Array.isArray(promise) ? promise : [promise]),
        aborted,
      ]),
    wait: (ms) => sleep(ms, undefined, { signal: controller.signal }),
  }
  const remaining = () =>
    deadline === Infinity
      ? 0
      : Math.max(1, Math.ceil(deadline - performance.now()))
  // Keep assertion/comparator options on the runner side. Only public screenshot options cross
  // the SDK boundary, together with the assertion's remaining deadline and cancellation.
  const screenshotOptions = Object.fromEntries(
    pixelOptions
      .filter((key) => options[key] !== undefined)
      .map((key) => [key, options[key]]),
  )
  const rafraf = async (subject, ms) => {
    if (!ms) return
    await progress.race(
      Promise.all([
        subject === page
          ? page.evaluate(
              () =>
                new Promise((resolve) =>
                  requestAnimationFrame(() => requestAnimationFrame(resolve)),
                ),
              undefined,
              { timeout: remaining(), signal: controller.signal },
            )
          : subject.evaluate(
              (element) =>
                new Promise((resolve) => {
                  const view = element.ownerDocument.defaultView
                  view.requestAnimationFrame(() =>
                    view.requestAnimationFrame(resolve),
                  )
                }),
              undefined,
              { timeout: remaining(), signal: controller.signal },
            ),
        progress.wait(ms),
      ]),
    )
  }
  const capture = async (subject, ms = 0) => {
    controller.signal.throwIfAborted()
    await rafraf(subject, ms)
    controller.signal.throwIfAborted()
    return progress.race(
      subject.screenshot({
        ...screenshotOptions,
        timeout: remaining(),
        signal: controller.signal,
      }),
    )
  }
  const frame = {
    rafrafTimeout: (_progress, ms) => rafraf(page, ms),
    isNonRetriableError: (error) =>
      controller.signal.aborted ||
      page.isClosed() ||
      [
        'invalid_input',
        'invalid_selector',
        'unsupported',
        'target_closed',
      ].includes(error.code),
  }
  const visualPage = {
    isClosed: () => page.isClosed(),
    mainFrame: () => frame,
    // Browser admission, locator checks and screenshot preparation stay in the public SDK
    // call below; this adapter supplies no reference-page action implementation.
    performActionPreChecks: async () => {},
    screenshotter: { screenshotPage: () => capture(page) },
  }
  const serverOptions = {
    ...options,
    locator: options.locator
      ? {
          selector: '',
          frame: {
            rafrafTimeoutScreenshotElementWithProgress: (
              _progress,
              _selector,
              ms,
            ) => capture(options.locator.neo, ms),
          },
        }
      : undefined,
  }
  try {
    // Reuse the pinned runner's actual stability loop, comparator and diff generation. It can
    // only reach Neo captures through the adapter above; no reference CDP capture is available.
    return await server.Page.prototype.expectScreenshot.call(
      visualPage,
      progress,
      serverOptions,
    )
  } catch (error) {
    if (!error.details) throw error
    const errorMessage = timedOut
      ? error.details.previous
        ? 'Failed to take two consecutive stable screenshots.'
        : `Timeout ${options.timeout}ms exceeded.`
      : error.details.customErrorMessage
    return { ...error.details, errorMessage, timedOut }
  } finally {
    clearTimeout(timer)
    options.signal?.removeEventListener('abort', cancel)
  }
}
