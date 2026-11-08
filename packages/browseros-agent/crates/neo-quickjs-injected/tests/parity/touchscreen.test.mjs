// page.touchscreen.tap through Neo and through Playwright on a fresh page. With touch:
// Playwright's side runs in a `hasTouch` context, Neo's on a plain tab of the default context
// (Neo has no context options); both return undefined, and the page sees the same touch and
// compatibility mouse events, a modifier held by page.keyboard on the touch (TA-006). Without
// touch: Playwright's default context refuses `touchscreen.tap`, Neo taps (DECIDED A1-3),
// asserted per engine.
//
// From Playwright's specs at the pin: tests/library/tap.spec.ts 'TA-001 should send all of the
// correct events @smoke' and 'TA-006 should work with modifiers'.
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  errorCategory,
  launchReference,
  playwrightVersion,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

const html = `<!doctype html><meta charset="utf-8"><title>touch</title>
<div id="pad" style="position:absolute;left:0;top:0;width:200px;height:200px"></div>
<script>
window.__events = []
for (const type of ['touchstart', 'touchend', 'pointerdown', 'mousedown', 'click'])
  document.addEventListener(type, (event) => window.__events.push({
    type, target: event.target.id || event.target.nodeName.toLowerCase(), isTrusted: event.isTrusted,
    alt: event.altKey ?? null, pointerType: event.pointerType ?? null }), true)
</script>`
const PROBE = 'window.__events'
const CATEGORY = `const errorCategory = ${errorCategory.toString()};\n`
const WITH_TOUCH = `${CATEGORY}
  const voids = [await page.touchscreen.tap(30, 30)]
  await page.keyboard.down('Alt')
  voids.push(await page.touchscreen.tap(40, 40))
  await page.keyboard.up('Alt')
  return voids.every((value) => value === undefined)
`
const WITHOUT_TOUCH = `${CATEGORY}
  try {
    await page.touchscreen.tap(30, 30)
    return 'tapped'
  } catch (error) {
    return { category: errorCategory(error), hasTouch: /hasTouch/.test(error.message) }
  }
`

test('touchscreen.tap sends the touch input of a hasTouch context', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/touch.html': html },
    })
    const url = `${origin}/touch.html`
    const { playwright, ...browser } = await launchReference(
      cleanups,
      browserBinary,
    )
    const [context] = playwright.contexts()
    const touch = await playwright.newContext({ hasTouch: true })
    cleanups.push(() => touch.close())
    const scripts = mkdtempSync(join(tmpdir(), 'neo-touchscreen-parity-'))
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

    await t.test('with touch: the same values and page events', async () => {
      const reference = await viaPlaywright(touch, WITH_TOUCH)
      const neo = await viaNeo('with-touch', WITH_TOUCH)
      assert.equal(reference.value, true)
      assert.equal(neo.value, reference.value)
      assert.deepEqual(neo.effect, reference.effect, 'what the page saw')
      assert.ok(
        neo.effect.some((event) => event.type === 'touchstart' && event.alt),
        'the held Alt reached the second touch',
      )
      assert.ok(
        neo.outcome.calls.every((call) => call.delivery === 'done'),
        JSON.stringify(neo.outcome.calls.map((call) => call.delivery)),
      )
    })

    await t.test('without touch: Playwright refuses, Neo taps', async () => {
      const reference = await viaPlaywright(context, WITHOUT_TOUCH)
      assert.deepEqual(reference.value, { category: 'Error', hasTouch: true })
      assert.deepEqual(reference.effect, [], 'Playwright sent nothing')
      const neo = await viaNeo('without-touch', WITHOUT_TOUCH)
      assert.equal(neo.value, 'tapped')
      assert.ok(neo.effect.some((event) => event.type === 'touchstart'))
    })
  } finally {
    await runCleanups(cleanups)
  }
})
