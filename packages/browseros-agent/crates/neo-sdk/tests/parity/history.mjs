// The runner the history parity files share (reload, goBack, goForward and
// waitForNavigation.test.mjs), on the shared harness: one BrowserOS neo launched as Playwright
// launches Chromium for its own tests, without the back-forward cache (`chromiumSwitches.ts`:
// `--disable-back-forward-cache`; `page-history.spec.ts` "BFCache should be disabled"), one
// fixture server, the pinned Playwright over connectOverCDP, and neo_run. Neo's handling of a
// cached document, which Playwright never meets, is neo-runtime's `history_e2e.rs`.
//
// Each case runs its script on two tabs opened the same way, so their history lists match: an
// `about:blank` tab that then navigates to the case's page. Playwright's script runs on the first,
// Neo's on the second, and each run reports what its script returned (a value, or a thrown
// error's kind) and what its page held afterwards (the case's `probe`). Never wording, call logs
// or timing.
//
// Every script gets `describe(response)` (its status, `ok()` and path, or `null`), `path()` (the
// page URL without its origin) and `settle(promise)` (its value, or `{ error: kind }`), so both
// engines report a call the same way.

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  eventually,
  findPage,
  launchBrowser,
  playwrightVersion,
  runCleanups,
  serveFixture,
  startNeo,
  waitForUrl,
} from './harness.mjs'

/** Playwright's `chromiumSwitches.ts`: `'--disable-back-forward-cache'`. */
const PLAYWRIGHT_SWITCHES = ['--disable-back-forward-cache']

const PRELUDE = `const origin = page.url().replace(/^([a-z]+:\\/\\/[^/]+).*$/, '$1')
const strip = (url) => url.startsWith(origin) ? url.slice(origin.length) : url
const path = () => strip(page.url())
const describe = (response) => response === null ? null : {
  status: response.status(),
  ok: response.ok(),
  path: strip(response.url()),
}
const settle = (promise) => promise.then(
  (value) => value,
  (error) => ({
    error: error.name === 'TimeoutError' ? 'timeout'
      : /has been closed/.test(error.message) ? 'closed'
      : error.name,
  }),
)
`

/**
 * Serves `pages` (`/slow` answers after `slowMs`), launches the browser and connects both
 * engines, then calls `body` with `run(c)`, which runs case `c` (`path`, `source`, optional
 * `probe`) on both engines, Playwright's tab first.
 */
export async function withHistory(t, { pages, slowMs = 0 }, body) {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages, slowMs })
    const browser = await launchBrowser(
      cleanups,
      browserBinary,
      PLAYWRIGHT_SWITCHES,
    )
    const cdp = await connectCdp(cleanups, browser.port)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-history-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    let next = 1
    await body(async (c) => {
      const url = `${origin}${c.path}`
      const source = PRELUDE + c.source
      const index = next++
      return {
        playwright: await onTab(cdp, url, c, (tab) =>
          viaPlaywright(context, tab, source),
        ),
        neo: await onTab(cdp, url, c, (tab) =>
          viaNeo(browser.port, neoRun, scripts, index, tab, source),
        ),
      }
    })
  } finally {
    await runCleanups(cleanups)
  }
}

/** Opens an `about:blank` tab, navigates it to `url`, waits until it loaded, runs `script` on
 * it, reads the case's probe, and closes it. */
async function onTab(cdp, url, c, script) {
  const { targetId } = await cdp.send('Target.createTarget', {
    url: 'about:blank',
  })
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
    await waitForUrl(cdp, sessionId, 'about:blank')
    await cdp.send('Page.navigate', { url }, sessionId)
    await waitForUrl(cdp, sessionId, url)
    await eventually(
      async () => (await evaluate('document.readyState')) === 'complete',
      `${url} loaded`,
    )
    const run = await script({ targetId })
    const probe = c.probe ? await evaluate(c.probe) : undefined
    return { ...run, probe }
  } finally {
    await cdp.send('Target.closeTarget', { targetId })
  }
}

/** Runs the script as Playwright does, with `page` in scope. */
async function viaPlaywright(context, { targetId }, source) {
  const page = await findPage(context, targetId)
  const AsyncFunction = (async () => {}).constructor
  const result = await new AsyncFunction('page', source)(page).then(
    (value) => ({ value }),
    (error) => ({ error: error.name }),
  )
  return { result }
}

/** Runs the script through neo_run. */
async function viaNeo(port, neoRun, scripts, index, { targetId }, source) {
  const scriptPath = join(scripts, `case-${index}.js`)
  writeFileSync(scriptPath, source)
  const neo = startNeo(neoRun, port, targetId, scriptPath)
  await neo.connected
  const outcome = await neo.outcome
  const result =
    outcome.status === 'ok'
      ? { value: outcome.value }
      : { error: outcome.error?.name }
  return { result, outcome }
}

/**
 * Runs `cases` as subtests: Playwright's result must be the case's `expected` (so each case pins
 * real behavior), Neo's must equal Playwright's, and so must the pages' probes. A case with
 * `neoExpected` is a recorded difference: Neo's result must be exactly that instead.
 */
export async function compareCases(t, cases, options) {
  await withHistory(t, options, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c)
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        if (c.neoExpected)
          assert.deepEqual(
            neo.result,
            { value: c.neoExpected },
            'Neo difference',
          )
        else
          assert.deepEqual(
            neo.result,
            playwright.result,
            `Neo vs Playwright: ${JSON.stringify(neo.outcome?.error ?? null)}`,
          )
        assert.deepEqual(neo.probe, playwright.probe, 'what each page held')
      })
    }
  })
}
