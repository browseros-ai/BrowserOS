// dispatchEvent on Page, Frame and Locator waits for the element, then dispatches an untrusted
// event built as Playwright's `InjectedScript.dispatchEvent` builds it: the event class its type
// names (MouseEvent, KeyboardEvent, WheelEvent, a plain Event for an unknown type), bubbling,
// cancelable and composed unless eventInit says otherwise, and eventInit's plain values. Page
// and Frame take the first of two matches; a strict locator, a missing element at its timeout
// and an invalid selector reject by kind. An element inserted 500 ms after the `#later` click
// is waited for, and a dispatch after a navigation reaches the new document.
//
// From Playwright's specs at the pin (tests/page/page-dispatchevent.spec.ts at 26137face):
// 'DE-001 should dispatch click event @smoke', 'DE-002 should dispatch click event properties',
// 'DE-003 should dispatch click svg', 'DE-004 should dispatch click on a span with an inline
// element inside', 'DE-005 should dispatch click after navigation' and 'DE-013 should dispatch
// wheel event'. Values Neo refuses in eventInit (a Date, NaN, a function, a typed array) are
// Neo-only: neo-runtime's tests/element_ops_e2e.rs.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('dispatchEvent dispatches the event Playwright builds, on the element it names', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/dispatch-event.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.dispatchEvent('#button', 'click') === undefined,
        await page.locator('#circle').dispatchEvent('click') === undefined,
        await page.locator('#span').dispatchEvent('click', undefined, null) === undefined,
        await page.locator('#button').dispatchEvent('click', { detail: 3, clientX: 12, button: 2 }) === undefined,
        await page.locator('#button').dispatchEvent('keydown', { key: 'a', bubbles: false, skipped: undefined }) === undefined,
        await page.locator('#button').dispatchEvent('wheel', { deltaY: 120 }, { timeout: 5000 }) === undefined,
        await page.locator('#button').dispatchEvent('ping', { cancelable: false, composed: false }) === undefined,
        await page.dispatchEvent('.dup', 'click') === undefined,
        await page.mainFrame().dispatchEvent('.dup', 'click', { detail: 1 }, null) === undefined,
      ]
      await page.goto(page.url())
      voids.push(await page.dispatchEvent('#button', 'click') === undefined)
      await page.click('#later')
      voids.push(await page.locator('#late').dispatchEvent('click') === undefined)
      const failures = {}
      for (const [name, act] of [
        ['strict', () => page.locator('.dup').dispatchEvent('click')],
        ['missing', () => page.locator('#missing').dispatchEvent('click', {}, { timeout: 500 })],
        ['invalid', () => page.dispatchEvent('[', 'click')],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      return { voids, failures }
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(neo, playwright, 'browser effect')
      const event = (fields) => ({
        type: 'click',
        kind: 'MouseEvent',
        isTrusted: false,
        bubbles: true,
        cancelable: true,
        composed: true,
        detail: 0,
        clientX: 0,
        button: 0,
        key: null,
        deltaY: null,
        ...fields,
      })
      assert.deepEqual(playwright.events, [
        event({ target: 'button' }),
        event({ target: 'circle' }),
        event({ target: 'span' }),
        event({ target: 'button', detail: 3, clientX: 12, button: 2 }),
        event({
          type: 'keydown',
          kind: 'KeyboardEvent',
          target: 'button',
          bubbles: false,
          clientX: null,
          button: null,
          key: 'a',
        }),
        event({
          type: 'wheel',
          kind: 'WheelEvent',
          target: 'button',
          deltaY: 120,
        }),
        event({
          type: 'ping',
          kind: 'Event',
          target: 'button',
          cancelable: false,
          composed: false,
          detail: null,
          clientX: null,
          button: null,
        }),
        event({ target: 'dup-one' }),
        event({ target: 'dup-one', detail: 1 }),
        event({ target: 'button' }),
        // page.click's trusted click, which Chromium dispatches as a PointerEvent.
        event({
          target: 'later',
          kind: 'PointerEvent',
          isTrusted: true,
          detail: 1,
          clientX: playwright.events[10]?.clientX,
        }),
        event({ target: 'late' }),
      ])
    },
  })
  assert.deepEqual(outcome.value, {
    voids: Array(11).fill(true),
    failures: {
      strict: 'Error:strict',
      missing: 'TimeoutError',
      invalid: 'Error',
    },
  })
})

/** dispatchEvent inside frames, on tabs of frame-top.html (frames.mjs): the button of the frame
 * a locator names, same-site and cross-site, and a missing iframe at its timeout. */
const frameCases = [
  {
    name: 'dispatchEvent reaches the button of the frame it names',
    source: `return {
      same: await settle(() => page.frameLocator('#same').locator('#send').dispatchEvent('click')),
      cross: await settle(() => page.frameLocator('#login-iframe').locator('#send').dispatchEvent('click', { detail: 2 })),
      missingFrame: await settle(() => page.frameLocator('#nope').locator('#send').dispatchEvent('click', {}, { timeout: 500 })),
    }`,
    expected: {
      same: 'undefined',
      cross: 'undefined',
      missingFrame: { error: 'timeout', name: 'TimeoutError' },
    },
    effect: (effect) => {
      const { same, cross } = assertDecoysUntouched(effect, ['same', 'cross'])
      for (const frame of [same, cross])
        assert.deepEqual(frame.events, [
          { type: 'click', target: 'send', trusted: false },
        ])
    },
  },
]

test('dispatchEvent inside frames reaches the same button through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
