// Taps through Neo and through Playwright on fresh tests/pages/pointer-actions.html pages.
//
// With touch: Playwright runs the script in a `hasTouch` context, Neo on a plain tab of the
// default context (Neo has no context options; its tab is the user's). Every receiver returns
// undefined, and the page saw the same trusted touch, pointer and compatibility mouse events on
// the same elements; a disabled element is waited for (tap waits for enabled), a hidden and a
// covered one time out untouched, and a strict locator over two matches fails at once.
//
// Without touch: Playwright's default context refuses `tap` before any input. Neo taps anyway
// (DECIDED A1-3 in decisions-pw-dom-input.md, a `Deviation:` in neo-runtime's click.rs):
// asserted here, so the difference stays visible.
//
// From Playwright's specs at the pin: tests/library/tap.spec.ts 'TA-001 should send all of the
// correct events @smoke' and 'TA-008 should wait until an element is visible to tap it'.
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, failed } from './frames.mjs'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  errorCategory,
  launchBrowser,
  playwrightVersion,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

const html = readFileSync(
  new URL('../pages/pointer-actions.html', import.meta.url),
  'utf8',
)
const PROBE = 'window.__facts()'
/** The harness's error kinds, defined in each script so both engines read errors alike. */
const ERROR_CATEGORY = `const errorCategory = ${errorCategory.toString()};\n`

const WITH_TOUCH = `${ERROR_CATEGORY}
  const voids = [
    await page.locator('#target').tap() === undefined,
    await page.tap('.dup', null) === undefined,
    await page.mainFrame().tap('#other', { position: { x: 4, y: 4 } }) === undefined,
  ]
  const failures = {}
  for (const [name, act] of [
    ['strict', () => page.locator('.dup').tap()],
    ['disabled', () => page.locator('#disabled').tap({ timeout: 500 })],
    ['hidden', () => page.locator('#hidden').tap({ timeout: 500 })],
    ['covered', () => page.locator('#covered').tap({ timeout: 1000 })],
  ]) {
    try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
  }
  return { voids, failures }
`

const WITHOUT_TOUCH = `${ERROR_CATEGORY}
  try {
    await page.locator('#target').tap({ timeout: 2000 })
    return 'tapped'
  } catch (error) {
    return { category: errorCategory(error), hasTouch: /hasTouch/.test(error.message) }
  }
`

test('tap sends the touch input of a hasTouch context, and Neo taps without one', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/tap.html': html },
    })
    const url = `${origin}/tap.html`
    const browser = await launchBrowser(cleanups, browserBinary)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const touch = await playwright.newContext({ hasTouch: true })
    cleanups.push(() => touch.close())
    const scripts = mkdtempSync(join(tmpdir(), 'neo-tap-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))

    const viaPlaywright = async (owner, source) => {
      const page = await owner.newPage()
      try {
        await page.goto(url)
        const AsyncFunction = (async () => {}).constructor
        const value = await new AsyncFunction('page', source)(page)
        return { value, effect: await page.evaluate(PROBE) }
      } finally {
        await page.close()
      }
    }
    const viaNeo = async (name, source) => {
      const page = await context.newPage()
      try {
        await page.goto(url)
        const session = await context.newCDPSession(page)
        const { targetInfo } = await session.send('Target.getTargetInfo')
        await session.detach()
        const script = join(scripts, `${name}.js`)
        writeFileSync(script, source)
        const neo = startNeo(neoRun, browser.port, targetInfo.targetId, script)
        await neo.connected
        const outcome = await neo.outcome
        assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
        return {
          outcome,
          value: outcome.value,
          effect: await page.evaluate(PROBE),
        }
      } finally {
        await page.close()
      }
    }

    await t.test(
      'with touch: the same values and the same page events',
      async () => {
        const reference = await viaPlaywright(touch, WITH_TOUCH)
        const neo = await viaNeo('with-touch', WITH_TOUCH)
        assert.deepEqual(reference.value, {
          voids: [true, true, true],
          failures: {
            strict: 'Error:strict',
            disabled: 'TimeoutError',
            hidden: 'TimeoutError',
            covered: 'TimeoutError',
          },
        })
        assert.deepEqual(neo.value, reference.value, 'public return values')
        assert.deepEqual(neo.effect, reference.effect, 'what the page saw')
        assert.ok(
          neo.effect.events.some(
            (event) => event.type === 'touchstart' && event.target === 'target',
          ),
          'a real touch reached the element',
        )
        assert.deepEqual(
          neo.outcome.calls.map((call) => call.delivery),
          ['done', 'done', 'done', 'notSent', 'notSent', 'notSent', 'notSent'],
          "Neo's delivery records",
        )
      },
    )

    await t.test('without touch: Playwright refuses, Neo taps', async () => {
      const reference = await viaPlaywright(context, WITHOUT_TOUCH)
      assert.deepEqual(reference.value, { category: 'Error', hasTouch: true })
      assert.deepEqual(reference.effect.events, [], 'Playwright sent nothing')
      const neo = await viaNeo('without-touch', WITHOUT_TOUCH)
      assert.equal(neo.value, 'tapped')
      assert.deepEqual(
        neo.effect.events
          .filter((event) => event.target === 'target')
          .map((event) => event.type),
        [
          'pointerdown',
          'touchstart',
          'pointerup',
          'touchend',
          'mouseover',
          'mouseenter',
          'mousemove',
          'mousedown',
          'mouseup',
          'click',
        ],
      )
    })
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * Taps inside frames, on tabs of frame-top.html (frames.mjs), which run in the default context:
 * Playwright refuses each tap there, and Neo taps (DECIDED A1-3). The touch lands on the Send
 * button of the cross-site out-of-process frame and of the same-origin frame, which each get
 * one trusted click from it, and no decoy frame sees one.
 */
const frameCases = [
  {
    name: 'a tap inside frames reaches the Send button of each frame and no decoy',
    source: `return {
      same: await settle(() => page.frameLocator('#same').locator('#send').tap()),
      cross: await settle(() => page.frameLocator('#login-iframe').locator('#send').tap()),
    }`,
    expected: { same: failed('error'), cross: failed('error') },
    effect: (effect) => assertDecoysUntouched(effect, []),
    expectedNeo: { same: 'undefined', cross: 'undefined' },
    neoEffect: (effect) => {
      const touched = assertDecoysUntouched(effect, ['same', 'cross'])
      for (const [who, facts] of Object.entries(touched))
        assert.deepEqual(
          facts.events.filter((event) => event.type === 'click'),
          [{ type: 'click', target: 'send', trusted: true }],
          `${who}'s tap`,
        )
    },
    deliveries: { tap: ['done', 'done'] },
  },
]

test('tap inside frames: Playwright refuses without hasTouch, Neo taps the right frame', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
