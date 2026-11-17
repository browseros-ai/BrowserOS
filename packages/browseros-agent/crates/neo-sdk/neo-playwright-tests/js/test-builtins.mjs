/** Neo: Supplies the pinned upstream test environment, without implementing browser APIs. */

/** Playwright: `packages/injected/src/utilityScript.ts` `UtilityScript.constructor`: `(global as any).builtins = this.builtins;` */
function exposeBuiltins(global) {
  global.builtins = global.__pwClock
    ? global.__pwClock.builtins
    : {
        setTimeout: global.setTimeout?.bind(global),
        clearTimeout: global.clearTimeout?.bind(global),
        setInterval: global.setInterval?.bind(global),
        clearInterval: global.clearInterval?.bind(global),
        requestAnimationFrame: global.requestAnimationFrame?.bind(global),
        cancelAnimationFrame: global.cancelAnimationFrame?.bind(global),
        requestIdleCallback: global.requestIdleCallback?.bind(global),
        cancelIdleCallback: global.cancelIdleCallback?.bind(global),
        performance: global.performance,
        Intl: global.Intl,
        Date: global.Date,
        AbortSignal: global.AbortSignal,
        Temporal: global.Temporal,
      }
}

/** Neo: One source initializes current documents and subsequent document epochs identically. */
const SOURCE = `(${exposeBuiltins})(globalThis)`

/** Neo: Uses only the owned infrastructure session; omitting worldName selects the main world. */
export async function installTestBuiltins(cdp, sessionId) {
  await cdp.send(
    'Page.addScriptToEvaluateOnNewDocument',
    { source: SOURCE, runImmediately: true },
    sessionId,
  )
  const result = await cdp.send(
    'Runtime.evaluate',
    { expression: SOURCE, returnByValue: true },
    sessionId,
  )
  if (result.exceptionDetails)
    throw new Error(
      'Upstream test builtins installation threw in the main world',
    )
}
