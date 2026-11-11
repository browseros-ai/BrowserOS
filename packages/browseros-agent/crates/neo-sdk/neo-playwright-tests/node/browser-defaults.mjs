/** Verify the real profile and the native tab creation path before any SDK attaches. */
import { Cdp } from '../js/browser.mjs'
import { cleanupCdp } from './cdp-cleanup.mjs'

/** Playwright's default viewport, which every engine's pages must start with. */
const VIEWPORT = { width: 1280, height: 720 }

export async function verifyViewport(port) {
  const response = await fetch(`http://127.0.0.1:${port}/json/version`)
  const cdp = await Cdp.connect((await response.json()).webSocketDebuggerUrl)
  let nativeTarget
  async function read(targetId) {
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    try {
      const { result, exceptionDetails } = await cdp.send(
        'Runtime.evaluate',
        {
          expression:
            '({ width: innerWidth, height: innerHeight, deviceScaleFactor: devicePixelRatio, locale: navigator.language, offline: !navigator.onLine, hasTouch: navigator.maxTouchPoints > 0, isMobile: /Mobile/.test(navigator.userAgent), javaScriptEnabled: true, colorScheme: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light", contrast: matchMedia("(prefers-contrast: no-preference)").matches ? "no-preference" : "other", forcedColors: matchMedia("(forced-colors: active)").matches ? "active" : "none", reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches ? "reduce" : "no-preference" })',
          returnByValue: true,
        },
        sessionId,
      )
      if (exceptionDetails)
        throw new Error(
          `harness-error: viewport probe failed: ${JSON.stringify(exceptionDetails)}`,
        )
      return result.value
    } finally {
      await cleanupCdp(cdp, 'Target.detachFromTarget', { sessionId })
    }
  }
  async function measure(targetId) {
    const value = await read(targetId)
    if (
      value?.width !== VIEWPORT.width ||
      value?.height !== VIEWPORT.height ||
      value?.deviceScaleFactor !== 1
    )
      throw new Error(
        `harness-error: default viewport must be 1280x720@1: ${JSON.stringify(value)}`,
      )
    return value
  }
  try {
    const { targetInfos } = await cdp.send('Target.getTargets')
    const launch = targetInfos.find((target) => target.type === 'page')
    if (!launch)
      throw new Error('harness-error: no launch page for viewport verification')
    const fitted = await fitWindow(cdp, launch.targetId, read)
    const initial = await measure(launch.targetId)
    const { tab } = await cdp.send('Browser.createTab', {
      url: 'about:blank',
      background: true,
    })
    nativeTarget = tab.targetId
    return { fitted, initial, native: await measure(nativeTarget) }
  } finally {
    try {
      if (nativeTarget)
        await cleanupCdp(cdp, 'Target.closeTarget', { targetId: nativeTarget })
    } finally {
      await cdp.close()
    }
  }
}

/**
 * BrowserOS's window chrome (side UI, toolbar) changes size between releases: 240x81 px on
 * 0.52.1.1, 240x47 px on 0.52.2. The launch `--window-size` is only a first guess, so the
 * window grows or shrinks by what the chrome took until the page area is exactly
 * Playwright's default. Native tabs open in the same window. `measure` still gates both tabs.
 */
async function fitWindow(cdp, targetId, read) {
  const page = await read(targetId)
  if (page?.width === VIEWPORT.width && page?.height === VIEWPORT.height)
    return null
  const { windowId, bounds } = await cdp.send('Browser.getWindowForTarget', {
    targetId,
  })
  const resized = {
    width: bounds.width + VIEWPORT.width - page.width,
    height: bounds.height + VIEWPORT.height - page.height,
  }
  await cdp.send('Browser.setWindowBounds', { windowId, bounds: resized })
  return {
    window: { width: bounds.width, height: bounds.height },
    page: { width: page.width, height: page.height },
    resized,
  }
}
