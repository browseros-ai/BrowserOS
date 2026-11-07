// Clicks through Neo and through real Playwright (playwright-core 1.64.1) on one BrowserOS neo,
// each case on fresh tabs serving the same page at the same URL: Playwright runs the case's
// script on its own tab through connectOverCDP, Neo runs it on another through neo_run, and the
// test compares what each script saw (its value, or its error's kind) and what each page's
// listeners saw (which events reached which element). It never compares an error's wording, the
// call log, click coordinates or retry timing.
//
// Two differences are deliberate (neo-runtime src/click.rs) and asserted, not hidden: Neo's
// trial sends no input, where Playwright's presses into an interceptor whose blocked events the
// page's own window listeners still see; and under noWaitAfter Neo still reads the press's
// hit-target verdict and presses again after a block, where Playwright resolves on the blocked
// press.
//
// The pages are tests/pages/; neo-runtime's click_e2e.rs runs them on Neo alone.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  runCleanups,
  serveFixture,
  skip,
  sleep,
  startNeo,
  waitForUrl,
} from './harness.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const page = (file) => readFileSync(join(here, '../pages', file), 'utf8')
const SUBMIT = "page.getByRole('button', { name: 'Submit' })"
/** When a case closes its tab, counted from the script's start. */
const CLOSE_AFTER_MS = 500
/** What a page's event recorder saw, without coordinates. */
const EVENTS =
  'window.__events.map(({ type, target, isTrusted, button, detail }) => ({ type, target, isTrusted, button, detail }))'

/**
 * Each case's page, script, what to read from the page after the run (`probe`), and the delivery
 * Neo's one record must report. `closeAfterMs` closes the tab while the script runs. A deliberate
 * difference names what each engine's page must show (`pages`) and what both scripts return
 * (`returns`).
 */
const cases = [
  {
    name: 'ok',
    html: page('ok.html'),
    source: `const result = await ${SUBMIT}.click()\nreturn { undefined: result === undefined, url: page.url() }`,
    probe: EVENTS,
    delivery: 'done',
  },
  {
    name: 'strict',
    html: page('strict.html'),
    source: `await ${SUBMIT}.click()`,
    probe: EVENTS,
    delivery: 'notSent',
  },
  {
    name: 'hidden',
    html: page('hidden.html'),
    source: `await ${SUBMIT}.click({ timeout: 500 })`,
    probe: EVENTS,
    delivery: 'notSent',
  },
  {
    name: 'covered',
    html: page('covered.html'),
    source: `await ${SUBMIT}.click({ timeout: 1000 })`,
    probe: EVENTS,
    delivery: 'notSent',
  },
  {
    name: 'disabled',
    html: page('disabled.html'),
    source: `await ${SUBMIT}.click({ timeout: 500 })`,
    probe: EVENTS,
    delivery: 'notSent',
  },
  {
    name: 'dialog',
    html: page('dialog.html'),
    source: `await ${SUBMIT}.click()\nreturn 'clicked'`,
    probe: 'window.__answer',
    delivery: 'done',
  },
  {
    name: 'blocked-once',
    html: page('blocked-once.html'),
    source: `await ${SUBMIT}.click()\nreturn 'clicked'`,
    probe: 'window.__clicks',
    delivery: 'done',
  },
  {
    // Both resolve. Playwright skips the verdict under noWaitAfter, so the press the overlay
    // caught counts as its click; Neo reads it, presses again, and the button's handler runs.
    name: 'blocked-once-no-wait-after',
    html: page('blocked-once.html'),
    source: `await ${SUBMIT}.click({ noWaitAfter: true })\nreturn 'clicked'`,
    probe: 'window.__clicks',
    returns: { value: 'clicked' },
    pages: { playwright: 0, neo: 1 },
    delivery: 'done',
  },
  {
    // Both resolve and neither button's handler runs (the hash stays empty). Playwright's
    // interceptor blocks the press only after the page's window listeners saw it; Neo sends none.
    name: 'trial',
    html: page('ok.html'),
    source: `await ${SUBMIT}.click({ trial: true })\nreturn page.url()`,
    probe: `[location.hash, ${EVENTS}]`,
    pages: {
      playwright: [
        '',
        [
          ['pointermove', -1, 0],
          ['pointerdown', 0, 0],
          ['pointerup', 0, 0],
          ['click', 0, 1],
        ].map(([type, button, detail]) => ({
          type,
          target: 'button',
          isTrusted: true,
          button,
          detail,
        })),
      ],
      neo: ['', []],
    },
    delivery: 'notSent',
  },
  {
    name: 'closed',
    html: page('hidden.html'),
    source: `try {\n  await ${SUBMIT}.click({ timeout: 20000 })\n} catch (error) {\n  return { name: error.name, closed: error.message.includes('has been closed') }\n}`,
    closeAfterMs: CLOSE_AFTER_MS,
    delivery: 'notSent',
  },
]

test('clicks read the same through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await withBoth(cases, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c.name)
        assert.deepEqual(
          kind(neo.result),
          kind(playwright.result),
          'what the script saw',
        )
        if (c.returns)
          assert.deepEqual(kind(playwright.result), c.returns, 'the value')
        if (c.pages) {
          assert.deepEqual(
            playwright.probe,
            c.pages.playwright,
            "what Playwright's page saw",
          )
          assert.deepEqual(neo.probe, c.pages.neo, "what Neo's page saw")
        } else {
          assert.deepEqual(neo.probe, playwright.probe, 'what the page saw')
        }
        assert.deepEqual(
          neo.outcome.calls.map((call) => call.delivery),
          [c.delivery],
          "Neo's delivery record",
        )
      })
    }
  })
})

/**
 * A script's value as is, or its error as a kind: the error's name, and for a plain `Error`
 * whether it was a strict mode violation. The wording is not compared.
 */
function kind(result) {
  if (!result.error) return result
  const { name, message } = result.error
  const strict = /strict mode violation/.test(message)
  return { error: { name, strict } }
}

/**
 * One browser, one fixture server serving every case's page at `/<name>.html`, Playwright
 * connected over CDP, and neo_run built. `body` gets `run(name)`, which runs that case on two
 * fresh tabs, Playwright's first, and returns both sides' results.
 */
async function withBoth(cases, body) {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const pages = Object.fromEntries(
      cases.map((c) => [`/${c.name}.html`, c.html]),
    )
    const origin = await serveFixture(cleanups, { pages })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-parity-scripts-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    await body(async (name) => {
      const c = cases.find((k) => k.name === name)
      const url = `${origin}/${c.name}.html`
      return {
        playwright: await viaPlaywright(context, cdp, url, c),
        neo: await viaNeo(cdp, browser.port, neoRun, scripts, url, c),
      }
    })
  } finally {
    await runCleanups(cleanups)
  }
}

/** Runs the case's script as Playwright does, with `page` and `context` in scope. */
async function viaPlaywright(context, cdp, url, c) {
  const page = await context.newPage()
  try {
    await page.goto(url)
    const session = await context.newCDPSession(page)
    const { targetInfo } = await session.send('Target.getTargetInfo')
    await session.detach()
    const AsyncFunction = (async () => {}).constructor
    const settled = new AsyncFunction('page', 'context', c.source)(
      page,
      context,
    ).then(
      (value) => ({ value }),
      (error) => ({ error: { name: error.name, message: error.message } }),
    )
    if (c.closeAfterMs) {
      await sleep(c.closeAfterMs)
      await cdp.send('Target.closeTarget', { targetId: targetInfo.targetId })
    }
    const result = await settled
    const probe = c.probe ? await page.evaluate(c.probe) : undefined
    return { result, probe }
  } finally {
    await page.close()
  }
}

/** Runs the case's script through neo_run on a tab of its own. */
async function viaNeo(cdp, port, neoRun, scripts, url, c) {
  const { targetId } = await cdp.send('Target.createTarget', { url })
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  try {
    await waitForUrl(cdp, sessionId, url)
    const scriptPath = join(scripts, `${c.name}.js`)
    writeFileSync(scriptPath, c.source)
    const neo = startNeo(neoRun, port, targetId, scriptPath)
    await neo.connected
    if (c.closeAfterMs) {
      await sleep(c.closeAfterMs)
      await cdp.send('Target.closeTarget', { targetId })
    }
    const outcome = await neo.outcome
    const result =
      outcome.status === 'ok'
        ? { value: outcome.value }
        : { error: outcome.error }
    let probe
    if (c.probe) {
      const reply = await cdp.send(
        'Runtime.evaluate',
        { expression: c.probe, returnByValue: true },
        sessionId,
      )
      probe = reply.result.value
    }
    return { outcome, result, probe }
  } finally {
    if (!c.closeAfterMs) await cdp.send('Target.closeTarget', { targetId })
  }
}
