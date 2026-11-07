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
// wheel event'. The event is built and dispatched in the page's own world, as Playwright's
// `mainWorld: true`, from `eventInit` as Playwright's serializer sends it: handles (a
// DataTransfer, a relatedTarget) reach main-world listeners, and values JSON cannot carry (a
// Date, NaN, a BigInt) arrive as Playwright's do ('DE-010 should dispatch drag drop events',
// 'DE-017 should throw if argument is from different frame').
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

const MAIN_WORLD = `<title>dispatch-main</title>
<button id="button">Button</button><div id="circle">Circle</div>
<script>
// BrowserOS neo's takeover bar is not the page's (frame-top.html explains); remove it on sight.
new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(
  document.documentElement,
  { childList: true, subtree: true },
)
// Main-world listeners: what each event carries when the page's own code reads it.
window.__seen = []
for (const type of ['click', 'focus', 'drop', 'ping', 'wheel'])
  document.getElementById('button').addEventListener(type, (event) =>
    window.__seen.push({
      type: event.type,
      kind: event.constructor.name,
      detail: event.detail === undefined ? 'undefined' : event.detail,
      related: event.relatedTarget ? event.relatedTarget.id : null,
      transfer: event.dataTransfer ? event.dataTransfer.getData('text/plain') : null,
      deltaY: 'deltaY' in event ? String(event.deltaY) : null,
    }))
window.__facts = () => ({ seen: window.__seen })
</script>`

test('dispatchEvent builds eventInit in the page world, handles included, as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: MAIN_WORLD,
    source: `
      const settle = async (act) => {
        try { await act(); return 'resolved' } catch (error) { return errorCategory(error) }
      }
      const circle = await page.$('#circle')
      // DE-010: a DataTransfer made in the page, its data set there.
      const transfer = await page.evaluateHandle(() => {
        const data = new DataTransfer()
        data.setData('text/plain', 'moved')
        return data
      })
      const results = {
        relatedTarget: await settle(() => page.locator('#button').dispatchEvent('focus', { relatedTarget: circle })),
        dataTransfer: await settle(() => page.dispatchEvent('#button', 'drop', { dataTransfer: transfer })),
        // An unknown type is a plain Event, which keeps no detail: Playwright builds no CustomEvent.
        detailObject: await settle(() => page.locator('#button').dispatchEvent('ping', { detail: { nested: [1, 'a'], when: new Date(0) } })),
        dateDetail: await settle(() => page.mainFrame().dispatchEvent('#button', 'click', { detail: new Date(5) })),
        // WheelEvent's deltaY is a finite double: the page's constructor throws, in both engines.
        nanDelta: await settle(() => page.locator('#button').dispatchEvent('wheel', { deltaY: NaN })),
        bigintDetail: await settle(() => page.locator('#button').dispatchEvent('click', { detail: 1n })),
        functionDetail: await settle(() => page.locator('#button').dispatchEvent('click', { detail: () => 1 })),
      }
      await transfer.dispose()
      results.disposed = await settle(() => page.dispatchEvent('#button', 'drop', { dataTransfer: transfer }, { timeout: 1000 }))
      return results
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(neo, playwright, 'what the page saw')
      assert.deepEqual(
        playwright.seen.map((event) => [
          event.type,
          event.kind,
          event.related,
          event.transfer,
        ]),
        [
          ['focus', 'FocusEvent', 'circle', null],
          ['drop', 'DragEvent', null, 'moved'],
          ['ping', 'Event', null, null],
          ['click', 'MouseEvent', null, null],
        ],
      )
    },
  })
  assert.deepEqual(outcome.value, {
    relatedTarget: 'resolved',
    dataTransfer: 'resolved',
    detailObject: 'resolved',
    dateDetail: 'resolved',
    nanDelta: 'Error',
    bigintDetail: 'Error',
    functionDetail: 'Error',
    disposed: 'Error',
  })
})

/** dispatchEvent inside frames, on tabs of frame-top.html (frames.mjs): the button of the frame
 * a locator names, same-site and cross-site, and a missing iframe at its timeout. */
const frameCases = [
  {
    name: 'dispatchEvent takes a handle of the frame it dispatches in, not of another (DE-017)',
    source: `const child = page.frames().find((frame) => frame.name() === 'same')
    const own = await child.evaluateHandle(() => new DataTransfer())
    const foreign = await page.evaluateHandle(() => new DataTransfer())
    return {
      own: await settle(() => page.frameLocator('#same').locator('#send').dispatchEvent('drop', { dataTransfer: own })),
      foreign: await settle(() => page.frameLocator('#same').locator('#send').dispatchEvent('drop', { dataTransfer: foreign })),
    }`,
    expected: {
      own: 'undefined',
      foreign: { error: 'error', name: 'Error' },
    },
  },
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

// Playwright resolves `dispatchEvent`'s selector in the page's own world (`mainWorld: true`),
// whose injected script holds no `ai` snapshot, so an `aria-ref` from one matches nothing there
// and the call waits out its timeout; `click` resolves it in the utility world, where the
// snapshot was taken. Neo does the same.
test('dispatchEvent on an aria-ref times out in both engines, as click resolves it', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<button id="b" onclick="window.clicks = (window.clicks || 0) + 1">Go</button>
<script>new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement, { childList: true, subtree: true })</script>`,
    source: `
      const yaml = await page.locator('#b').ariaSnapshot({ mode: 'ai' })
      const ref = /\\[ref=([a-z0-9]+)\\]/.exec(yaml)?.[1]
      const settle = (call) => call.then(() => 'resolved', (error) => error.name)
      return {
        dispatch: await settle(page.locator('aria-ref=' + ref).dispatchEvent('click', {}, { timeout: 1500 })),
        click: await settle(page.locator('aria-ref=' + ref).click({ timeout: 1500 })),
        clicks: await page.evaluate(() => window.clicks || 0),
      }
    `,
    probe: '({ clicks: window.clicks || 0 })',
  })
  assert.deepEqual(outcome.value, {
    dispatch: 'TimeoutError',
    click: 'resolved',
    clicks: 1,
  })
})
