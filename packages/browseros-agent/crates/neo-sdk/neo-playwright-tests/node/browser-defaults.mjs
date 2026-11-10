/** Verify the real profile and the native tab creation path before any SDK attaches. */
import { Cdp } from '../js/browser.mjs'

export async function verifyViewport(port) {
  const response = await fetch(`http://127.0.0.1:${port}/json/version`)
  const cdp = await Cdp.connect((await response.json()).webSocketDebuggerUrl)
  let nativeTarget
  async function measure(targetId) {
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
      if (
        exceptionDetails ||
        result.value?.width !== 1280 ||
        result.value?.height !== 720 ||
        result.value?.deviceScaleFactor !== 1
      )
        throw new Error(
          `harness-error: default viewport must be 1280x720@1: ${JSON.stringify(result.value)}`,
        )
      return result.value
    } finally {
      await cdp.send('Target.detachFromTarget', { sessionId })
    }
  }
  try {
    const { targetInfos } = await cdp.send('Target.getTargets')
    const launch = targetInfos.find((target) => target.type === 'page')
    if (!launch)
      throw new Error('harness-error: no launch page for viewport verification')
    const initial = await measure(launch.targetId)
    const { tab } = await cdp.send('Browser.createTab', {
      url: 'about:blank',
      background: true,
    })
    nativeTarget = tab.targetId
    return { initial, native: await measure(nativeTarget) }
  } finally {
    try {
      if (nativeTarget)
        await cdp.send('Target.closeTarget', { targetId: nativeTarget })
    } finally {
      await cdp.close()
    }
  }
}
