// Playwright's `signal` option through Neo and through Playwright on fresh pages that count their
// clicks, once per receiver family (`locator.click`, `page.click`, `page.mainFrame().click`), a
// read (`page.textContent`) and a navigation (`page.goto`).
//
// An already-aborted signal: both engines reject with `AbortError` before anything is sent, so
// the page sees no click and stays where it was. A live signal that never aborts and `null` (no
// signal) both run every call. Since slice S Neo honours the signal as Playwright does, so the
// two engines' results and browser effects are compared directly. Never the error's wording.
// Aborts during a call: abort.test.mjs.
//
// The scripts build the second URL with a string replace: Neo's realm has no global `URL`.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).
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

const html = `<!doctype html><meta charset="utf-8"><title>signal</title>
<button id="target" type="button" onclick="window.__clicks++">Target</button>
<script>window.__clicks = 0</script>`
const PROBE = '({ clicks: window.__clicks, path: location.pathname })'

/** Runs each call with `signal` and records how it settled: `resolved` or the error's kind. A
 * live signal's run leaves out `goto`, which would leave the page that counts the clicks. */
const script = (
  signal,
  { goto = true } = {},
) => `const errorCategory = ${errorCategory.toString()};
  const signal = ${signal}
  const results = {}
  for (const [name, call] of [
    ['locator.click', () => page.locator('#target').click({ signal })],
    ['page.click', () => page.click('#target', { signal })],
    ['frame.click', () => page.mainFrame().click('#target', { signal })],
    ['page.textContent', () => page.textContent('#target', { signal })],
    ${goto ? "['page.goto', () => page.goto(page.url().replace(/[^/]*$/, 'elsewhere'), { signal })]," : ''}
  ]) {
    try {
      await call()
      results[name] = 'resolved'
    } catch (error) {
      results[name] = errorCategory(error)
    }
  }
  return results
`
const ABORTED = "{ aborted: true, reason: new Error('stop') }"
const LIVE =
  '{ aborted: false, addEventListener() {}, removeEventListener() {} }'
const NAMES = ['locator.click', 'page.click', 'frame.click', 'page.textContent']
const EVERY = (kind, names = [...NAMES, 'page.goto']) =>
  Object.fromEntries(names.map((name) => [name, kind]))

test('a signal is honoured on every receiver family', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/signal.html': html, '/elsewhere': '<title>elsewhere</title>' },
    })
    const url = `${origin}/signal.html`
    const { playwright, ...browser } = await launchReference(
      cleanups,
      browserBinary,
    )
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-signal-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const viaPlaywright = async (source) => {
      const page = await context.newPage()
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
        const path = join(scripts, `${name}.js`)
        writeFileSync(path, source)
        const neo = startNeo(neoRun, browser.port, targetInfo.targetId, path)
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
      'an aborted signal: both reject with AbortError, the page is untouched',
      async () => {
        const reference = await viaPlaywright(script(ABORTED))
        assert.deepEqual(reference.value, EVERY('AbortError'))
        assert.deepEqual(reference.effect, { clicks: 0, path: '/signal.html' })
        const neo = await viaNeo('aborted', script(ABORTED))
        assert.deepEqual(neo.value, reference.value)
        assert.deepEqual(neo.effect, reference.effect, 'what the page saw')
        assert.deepEqual(neo.outcome.calls, [], 'Neo sent no call')
      },
    )

    await t.test('null is no signal: both engines run the calls', async () => {
      const none = script('null', { goto: false })
      const reference = await viaPlaywright(none)
      assert.deepEqual(reference.value, EVERY('resolved', NAMES))
      assert.deepEqual(reference.effect, { clicks: 3, path: '/signal.html' })
      const neo = await viaNeo('null', none)
      assert.deepEqual(neo.value, reference.value)
      assert.deepEqual(neo.effect, reference.effect, 'what the page saw')
    })

    await t.test(
      'a live signal that never aborts: both engines run the calls',
      async () => {
        const live = script(LIVE, { goto: false })
        const reference = await viaPlaywright(live)
        assert.deepEqual(reference.value, EVERY('resolved', NAMES))
        assert.deepEqual(reference.effect, { clicks: 3, path: '/signal.html' })
        const neo = await viaNeo('live', live)
        assert.deepEqual(neo.value, reference.value)
        assert.deepEqual(neo.effect, reference.effect, 'what the page saw')
      },
    )
  } finally {
    await runCleanups(cleanups)
  }
})
