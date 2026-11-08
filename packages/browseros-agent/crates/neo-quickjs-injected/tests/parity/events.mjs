// The runner the event parity files share (console, dialog, pageError, waitForEvent,
// consoleMessages and eventEmitter.test.mjs), on the shared harness: one BrowserOS neo, one
// fixture server and one neo_run build per file. Each case runs its script through the pinned
// Playwright on a fresh tab (connectOverCDP, newPage, goto), disconnects Playwright, then runs
// the same script through Neo on a fresh tab of its own, opened as a user's tab and loaded
// before Neo attaches. Playwright is gone during Neo's turn because its server answers every
// dialog no Playwright client listens for on every tab it is attached to, Neo's included
// (server/dialog.ts `DialogManager.dialogDidOpen`), and a fresh connection per case leaves no
// listener of one case on the next case's context.
//
// Every script gets `errorCategory` (harness.mjs), `settle` and `path` first, so both engines
// report the same way: `settle(promise)` is `'undefined'`, `{ value }` or `{ error: kind }`,
// never the error's wording; `path(url)` drops the origin, whose port changes between runs.
//
// NEO_PARITY_REFERENCE_ONLY=1 runs only Playwright's turn, to check a case's expected values
// before Neo serves what it calls.

import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  connectReference,
  errorCategory,
  eventually,
  launchBrowser,
  openTab,
  playwrightVersion,
  referenceHalf,
  runCleanups,
  runInNeo,
  serveFixture,
} from './harness.mjs'

const referenceOnly = Boolean(process.env.NEO_PARITY_REFERENCE_ONLY)
/** How long the page probe may take on Neo's tab; a dialog left open would hold it forever. */
const PROBE_TIMEOUT_MS = 5_000

const PRELUDE = `const errorCategory = ${errorCategory.toString()};
const settle = (promise) => promise.then(
  (value) => (value === undefined ? 'undefined' : { value }),
  (error) => ({ error: errorCategory(error) }),
);
const path = (url) => url.replace(/^[a-z]+:\\/\\/[^/]+/, '');
`

/** A page of tests/pages. */
export function fixture(file) {
  return readFileSync(new URL(`../pages/${file}`, import.meta.url), 'utf8')
}

/**
 * Runs each case as a subtest of `t`. A case names the fixture `path` it opens, its `source`,
 * the value Playwright's script returns (`expected`), and optionally a `probe` expression read
 * from the page after the script with the value Playwright's page holds (`effect`). Neo must
 * return what Playwright returned and leave its page as Playwright's was. A case with `neo` in
 * place of `expected` runs through Neo alone, for what Playwright's Node process cannot survive;
 * `neo(outcome)` asserts Neo's `RunOutcome`. A case where Neo differs on purpose (a recorded
 * Deviation) keeps `expected` for Playwright and pins Neo's side with `deviation(outcome)`.
 */
export async function compareEvents(t, { pages, cases }) {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  if (referenceOnly) t.diagnostic('NEO_PARITY_REFERENCE_ONLY: Playwright only')
  const neoRun = referenceOnly ? undefined : buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-event-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    let next = 1
    const neo = { cdp, port: browser.port, neoRun, scripts }
    for (const c of cases) {
      const skip = referenceOnly && c.neo ? 'Neo only' : false
      await t.test(c.name, { skip }, async () => {
        const url = `${origin}${c.path}`
        const source = PRELUDE + c.source
        if (c.neo) {
          c.neo((await viaNeo(neo, next++, url, source, c.probe)).outcome)
          return
        }
        const playwright = await viaPlaywright(
          browser.port,
          url,
          source,
          c.probe,
        )
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        assert.deepEqual(playwright.effect, c.effect, "Playwright's page")
        if (referenceOnly) return
        const theirs = await viaNeo(neo, next++, url, source, c.probe)
        if (c.deviation) {
          c.deviation(theirs.outcome)
          return
        }
        assert.deepEqual(
          theirs.result,
          playwright.result,
          `Neo vs Playwright: ${JSON.stringify(theirs.outcome)}`,
        )
        assert.deepEqual(theirs.effect, playwright.effect, "Neo's page")
      })
    }
  } finally {
    await runCleanups(cleanups)
  }
}

/**
 * Runs the script as Playwright does, with `page` and `context` in scope, then disconnects
 * (`referenceHalf`).
 */
async function viaPlaywright(port, url, source, probe) {
  return await referenceHalf(async () => {
    const playwright = await connectReference(port)
    try {
      const [context] = playwright.contexts()
      const page = await context.newPage()
      await page.goto(url)
      const AsyncFunction = (async () => {}).constructor
      const result = await new AsyncFunction('page', 'context', source)(
        page,
        context,
      ).then(
        (value) => ({ value }),
        (error) => ({ error: { name: error.name, message: error.message } }),
      )
      const effect = probe ? await page.evaluate(probe) : undefined
      await page.close()
      return { result, effect }
    } finally {
      await playwright.close()
    }
  })
}

/** Runs the script through neo_run on a loaded tab of its own, with no other client on it. */
async function viaNeo(
  { cdp, port, neoRun, scripts },
  index,
  url,
  source,
  probe,
) {
  const { targetId, sessionId } = await openTab(cdp, url)
  try {
    await eventually(
      async () =>
        (await evaluate(cdp, sessionId, 'document.readyState')) === 'complete',
      `${url} loaded`,
    )
    const scriptPath = join(scripts, `case-${index}.js`)
    writeFileSync(scriptPath, source)
    const { outcome } = await runInNeo(neoRun, port, targetId, scriptPath)
    const result =
      outcome.status === 'ok'
        ? { value: outcome.value }
        : { error: outcome.error }
    const effect = probe ? await probeWithin(cdp, sessionId, probe) : undefined
    return { outcome, result, effect }
  } finally {
    // A script may have closed its tab itself.
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
  }
}

async function evaluate(cdp, sessionId, expression) {
  const { result } = await cdp.send(
    'Runtime.evaluate',
    { expression, returnByValue: true },
    sessionId,
  )
  return result.value
}

async function probeWithin(cdp, sessionId, probe) {
  let timer
  const late = new Promise((resolve) => {
    timer = setTimeout(
      () => resolve('the probe timed out: is a dialog still open?'),
      PROBE_TIMEOUT_MS,
    )
  })
  try {
    return await Promise.race([evaluate(cdp, sessionId, probe), late])
  } finally {
    clearTimeout(timer)
  }
}
