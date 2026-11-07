// The runner the wait parity files share (waitFor, waitForURL and waitForLoadState.test.mjs), on
// the shared harness: one BrowserOS neo, one fixture server, the pinned Playwright over
// connectOverCDP, and neo_run. Each case runs its script on two fresh tabs at the same URL,
// Playwright's first, then Neo's, and returns what each script returned and what each page held
// afterwards (the case's `probe`).
//
// Every script gets `settle` first, so both engines report a call the same way: `'undefined'`
// for a call that resolved `undefined`, `{ value }` for any other value, or its error's kind,
// never its wording.

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  browserBinary,
  buildNeoRun,
  eventually,
  launchReference,
  playwrightVersion,
  referenceHalf,
  runCleanups,
  serveFixture,
  startNeo,
  waitForUrl,
} from './harness.mjs'

const SETTLE = `const settle = (promise) => promise.then(
  (value) => (value === undefined ? 'undefined' : { value }),
  (error) => ({
    error:
      error.name === 'TimeoutError' ? 'timeout'
      : /strict mode violation/.test(error.message) ? 'strict'
      : /has been closed/.test(error.message) ? 'closed'
      : 'error',
  }),
)
`

/**
 * Serves `pages` (`/slow` answers after `slowMs`), launches the browser and connects both
 * engines, then calls `body` with `run(c)`, which runs case `c` on both engines. A case names
 * its `path`, its `source`, an optional `probe` expression, and `loaded: false` to start the
 * script once the page has committed instead of once it has loaded.
 */
export async function withBoth(t, { pages, slowMs = 0 }, body) {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages, slowMs })
    const { playwright, cdp, ...browser } = await launchReference(
      cleanups,
      browserBinary,
      { cdp: true },
    )
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-wait-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    let next = 1
    await body(async (c) => {
      const url = `${origin}${c.path}`
      const source = SETTLE + c.source
      return {
        playwright: await referenceHalf(() =>
          viaPlaywright(context, url, source, c),
        ),
        neo: await viaNeo(cdp, browser.port, neoRun, scripts, next++, {
          url,
          source,
          c,
        }),
      }
    })
  } finally {
    await runCleanups(cleanups)
  }
}

/** Runs the script as Playwright does, with `page` and `context` in scope. */
async function viaPlaywright(context, url, source, c) {
  const page = await context.newPage()
  try {
    await page.goto(url, { waitUntil: c.loaded === false ? 'commit' : 'load' })
    const AsyncFunction = (async () => {}).constructor
    const result = await new AsyncFunction('page', 'context', source)(
      page,
      context,
    ).then(
      (value) => ({ value }),
      (error) => ({ error: { name: error.name, message: error.message } }),
    )
    const probe = c.probe ? await page.evaluate(c.probe) : undefined
    return { result, probe }
  } finally {
    await page.close()
  }
}

/** Runs the script through neo_run on a tab of its own, opened as a user's tab would be. */
async function viaNeo(cdp, port, neoRun, scripts, index, { url, source, c }) {
  const { targetId } = await cdp.send('Target.createTarget', { url })
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  const evaluate = async (expression) =>
    (
      await cdp.send(
        'Runtime.evaluate',
        { expression, returnByValue: true },
        sessionId,
      )
    ).result.value
  try {
    await waitForUrl(cdp, sessionId, url)
    if (c.loaded !== false)
      await eventually(
        async () => (await evaluate('document.readyState')) === 'complete',
        `${url} loaded`,
      )
    const scriptPath = join(scripts, `case-${index}.js`)
    writeFileSync(scriptPath, source)
    const neo = startNeo(neoRun, port, targetId, scriptPath)
    await neo.connected
    const outcome = await neo.outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    const probe = c.probe ? await evaluate(c.probe) : undefined
    return { outcome, result: { value: outcome.value }, probe }
  } finally {
    await cdp.send('Target.closeTarget', { targetId })
  }
}
