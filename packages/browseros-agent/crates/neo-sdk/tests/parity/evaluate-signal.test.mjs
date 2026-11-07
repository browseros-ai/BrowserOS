// A `signal` on the evaluation and handle methods Neo serves behaves as Playwright's (slice S).
// A page counter that every page function bumps shows whether the call took effect. One case per
// receiver family: Page, Frame, Locator, JSHandle/ElementHandle.
//
// - `signal: null` is no signal: the call runs exactly as without the option, in both engines.
// - Where Playwright takes a signal (`locator.evaluate`, `evaluateHandle` and `elementHandle`,
//   whose element wait sends it, and `waitForFunction`), it rejects an already-aborted one
//   with `AbortError` before the call runs (tests/page/locator-wait-for-function.spec.ts at
//   26137face, LO-192 "should abort via already-aborted signal"). Compared directly: both
//   reject, the counter stays 0.
// - Where Playwright reads no signal (the page, frame and handle evaluations, and `$`, all sent
//   with `kNoTimeout`), it ignores one and runs the call; so does Neo. Compared directly.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html =
  '<title>signal</title><p id="p">text</p><script>window.__count = 0</script>'

const SIGNAL = `
const signal = AbortSignal.abort('stop')
const bump = () => { window.__count++; return 'ran' }
const attempt = async (call) => {
  try {
    await call()
    return 'resolved'
  } catch (error) {
    return error instanceof Error ? error.name : 'threw a non-Error'
  }
}
`

test('methods that take a signal reject an aborted one before the page sees the call', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${SIGNAL}
      const frame = page.mainFrame()
      const locator = page.locator('#p')
      return {
        locatorEvaluate: await attempt(() => locator.evaluate(bump, undefined, { signal })),
        locatorEvaluateHandle: await attempt(() => locator.evaluateHandle(bump, undefined, { signal })),
        locatorElementHandle: await attempt(() => locator.elementHandle({ signal })),
        pageWaitForFunction: await attempt(() => page.waitForFunction(bump, undefined, { signal })),
        frameWaitForFunction: await attempt(() => frame.waitForFunction(bump, undefined, { signal })),
      }
    `,
    probe: 'window.__count',
  })
  assert.deepEqual(outcome.value, {
    locatorEvaluate: 'AbortError',
    locatorEvaluateHandle: 'AbortError',
    locatorElementHandle: 'AbortError',
    pageWaitForFunction: 'AbortError',
    frameWaitForFunction: 'AbortError',
  })
})

test('evaluations Playwright reads no signal on ignore one in both engines', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${SIGNAL}
      const handle = await page.evaluateHandle(() => document.querySelector('#p'))
      const frame = page.mainFrame()
      const outcomes = {
        pageEvaluate: await attempt(() => page.evaluate(bump, undefined, { signal })),
        pageEvaluateHandle: await attempt(() => page.evaluateHandle(bump, undefined, { signal })),
        frameEvaluate: await attempt(() => frame.evaluate(bump, undefined, { signal })),
        frameEvaluateHandle: await attempt(() => frame.evaluateHandle(bump, undefined, { signal })),
        handleEvaluate: await attempt(() => handle.evaluate(bump, undefined, { signal })),
        handleEvaluateHandle: await attempt(() => handle.evaluateHandle(bump, undefined, { signal })),
        pageQuery: await attempt(() => page.$('#p', { signal })),
        frameQuery: await attempt(() => frame.$('#p', { signal })),
      }
      await page.evaluate((recorded) => { window.__outcomes = recorded }, outcomes)
      return null
    `,
    probe: '({ count: window.__count, outcomes: window.__outcomes })',
    effects: ({ playwright, neo }) => {
      const each = (value) =>
        Object.fromEntries(
          [
            'pageEvaluate',
            'pageEvaluateHandle',
            'frameEvaluate',
            'frameEvaluateHandle',
            'handleEvaluate',
            'handleEvaluateHandle',
            'pageQuery',
            'frameQuery',
          ].map((key) => [key, value]),
        )
      assert.deepEqual(
        playwright,
        { count: 6, outcomes: each('resolved') },
        'Playwright',
      )
      assert.deepEqual(neo, playwright, 'Neo')
    },
  })
})

test('a null signal is no signal: the call runs as without the option', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${SIGNAL}
      const handle = await page.evaluateHandle(() => document.querySelector('#p'))
      const frame = page.mainFrame()
      const none = { signal: null }
      return {
        locatorEvaluate: await attempt(() => page.locator('#p').evaluate(bump, undefined, none)),
        pageEvaluate: await attempt(() => page.evaluate(bump, undefined, none)),
        frameEvaluate: await attempt(() => frame.evaluate(bump, undefined, none)),
        handleEvaluate: await attempt(() => handle.evaluate(bump, undefined, none)),
        locatorEvaluateHandle: await attempt(() => page.locator('#p').evaluateHandle(bump, undefined, none)),
        locatorElementHandle: await attempt(() => page.locator('#p').elementHandle(none)),
        pageWaitForFunction: await attempt(() => page.waitForFunction(bump, undefined, none)),
        frameWaitForFunction: await attempt(() => frame.waitForFunction(bump, undefined, none)),
        pageQuery: await attempt(() => page.$('#p', none)),
        frameQuery: await attempt(() => frame.$('#p', none)),
      }
    `,
    probe: 'window.__count',
  })
  assert.deepEqual(outcome.value, {
    locatorEvaluate: 'resolved',
    pageEvaluate: 'resolved',
    frameEvaluate: 'resolved',
    handleEvaluate: 'resolved',
    locatorEvaluateHandle: 'resolved',
    locatorElementHandle: 'resolved',
    pageWaitForFunction: 'resolved',
    frameWaitForFunction: 'resolved',
    pageQuery: 'resolved',
    frameQuery: 'resolved',
  })
})
