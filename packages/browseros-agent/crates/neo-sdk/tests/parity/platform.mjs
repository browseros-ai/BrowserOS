// What the platform parity files share (Browser, BrowserType, the `playwright` object, emulation
// and GC): one headless BrowserOS neo, real Playwright over connectOverCDP, and Neo through
// neo_run. Each case runs one script through both engines as `async (page, context, playwright)`,
// with Playwright's own `playwright` object on its side and Neo's realm parameter on Neo's, each
// on a fresh tab of the same URL, Playwright's turn first. A script reads its page through the
// fixture's own reporter (`tests/pages/emulation.html`), never through `page.evaluate`, so the
// same source runs in both.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  errorCategory,
  launchBrowser,
  openTab,
  playwrightVersion,
  runCleanups,
  runInNeo,
  serveFixture,
} from './harness.mjs'

const here = dirname(fileURLToPath(import.meta.url))

/** Playwright's `playwright` object, the module `chromium` came from (harness.mjs loads it). */
const playwrightDir = process.env.NEO_PARITY_PLAYWRIGHT
export const playwright = playwrightDir
  ? (await import(pathToFileURL(join(playwrightDir, 'index.mjs')).href)).default
  : (await import('playwright-core')).default

/** A fixture page of `tests/pages/`. */
export function fixturePage(file) {
  return readFileSync(join(here, '../pages', file), 'utf8')
}

/**
 * Helpers every script gets in both engines: `state()` reads the emulation fixture's reporter,
 * `settle(done)` polls it until `done(state)` or 3 s pass and returns the last state, and
 * `errorCategory` is harness.mjs's.
 */
const PRELUDE = `const errorCategory = ${errorCategory.toString()};
const state = async (p = page) => JSON.parse(await p.locator('#state').textContent())
const settle = async (done, p = page) => {
  const until = Date.now() + 3000
  for (;;) {
    const s = await state(p)
    if (done(s) || Date.now() > until) return s
    await new Promise((resolve) => setTimeout(resolve, 25))
  }
}
`

/**
 * Runs `body` with a browser, a fixture server serving `pages`, and the two engines:
 * `playwright(path, source)` and `neo(path, source)` each open a fresh tab on `origin + path`,
 * run `source`, and settle with `{ value }` or `{ error: { name, message } }`; Neo's also carries
 * the run's `outcome` and its tab's `targetId`. `cdp` is the test's own client, for reading what
 * a run left behind.
 */
export async function withPlatform(t, { pages = {} }, body) {
  t.diagnostic(`Playwright ${playwrightVersion}; ${playwrightDir ?? 'npm'}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-platform-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    let connected
    const connect = async () => {
      if (connected?.isConnected()) return connected
      connected = await chromium.connectOverCDP(
        `http://127.0.0.1:${browser.port}`,
      )
      const browserToClose = connected
      cleanups.push(() => browserToClose.close().catch(() => {}))
      return connected
    }
    let nextScript = 1
    const engines = {
      origin,
      cdp,
      port: browser.port,
      connect,
      async playwright(path, source) {
        const pw = await connect()
        const [context] = pw.contexts()
        const page = await context.newPage()
        await page.goto(`${origin}${path}`)
        const AsyncFunction = (async () => {}).constructor
        const run = new AsyncFunction(
          'page',
          'context',
          'playwright',
          `${PRELUDE}${source}`,
        )
        const result = await run(page, context, playwright).then(
          (value) => ({ value }),
          (error) => ({ error: { name: error.name, message: error.message } }),
        )
        if (pw.isConnected() && !page.isClosed())
          await page.close().catch(() => {})
        return result
      },
      async neo(path, source) {
        const { targetId } = await openTab(cdp, `${origin}${path}`)
        const file = join(scripts, `case-${nextScript++}.js`)
        writeFileSync(file, `${PRELUDE}${source}`)
        const result = await runInNeo(neoRun, browser.port, targetId, file)
        return { ...result, targetId }
      },
    }
    await body(engines)
  } finally {
    await runCleanups(cleanups)
  }
}

/** The value a run returned, or a description of how it failed instead, for assertions. */
export function outcomeValue(result) {
  return 'value' in result
    ? result.value
    : { failed: result.error, logs: result.outcome?.logs }
}
