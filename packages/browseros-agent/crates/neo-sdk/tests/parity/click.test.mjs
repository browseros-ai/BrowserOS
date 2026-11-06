// Clicks through Neo and through real Playwright (playwright-core 1.64.1) on one BrowserOS neo,
// each case on fresh tabs serving the same page at the same URL: Playwright runs the case's
// script on its own tab through connectOverCDP, Neo runs it on another through neo_run, and the
// test compares what each script saw and what each page's listeners saw.
//
// The golden cases (tests/goldens/click-goldens.json) are also held to the recorded golden, so a
// Playwright upgrade cannot move both sides at once unnoticed. The other cases are the ones
// neo-runtime's click_e2e.rs checks on Neo alone (a dialog, a press blocked after the hit check,
// a trial, a tab closed mid-click), here held to Playwright on the same pages
// (tests/pages/).
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
const goldens = JSON.parse(
  readFileSync(join(here, '../goldens/click-goldens.json'), 'utf8'),
).cases
const golden = (name) => goldens.find((g) => g.name === name)
const page = (file) => readFileSync(join(here, '../pages', file), 'utf8')
const SUBMIT = "page.getByRole('button', { name: 'Submit' })"
/** When a case closes its tab, counted from the script's start. */
const CLOSE_AFTER_MS = 500

/**
 * Neo's own cases. `probe` is read from each page after its run; `closeAfterMs` closes the
 * tab while the script runs.
 */
const neoCases = [
  {
    name: 'dialog',
    html: page('dialog.html'),
    source: `await ${SUBMIT}.click()\nreturn 'clicked'`,
    probe: 'window.__answer',
  },
  {
    name: 'blocked-once',
    html: page('blocked-once.html'),
    source: `await ${SUBMIT}.click()\nreturn 'clicked'`,
    probe: 'window.__clicks',
  },
  {
    name: 'trial',
    html: golden('ok').fixtureHtml,
    source: `await ${SUBMIT}.click({ trial: true })\nreturn page.url()`,
    probe: '[location.hash, window.__events]',
  },
  {
    name: 'closed',
    html: golden('hidden').fixtureHtml,
    source: `try {\n  await ${SUBMIT}.click({ timeout: 20000 })\n} catch (error) {\n  return { name: error.name, message: error.message }\n}`,
    closeAfterMs: CLOSE_AFTER_MS,
  },
]

test('golden clicks read the same through Neo, Playwright and the goldens', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await withBoth(
    goldens.map((g) => ({
      name: g.name,
      html: g.fixtureHtml,
      source: g.source,
      probe: 'window.__events',
    })),
    async (run) => {
      for (const g of goldens) {
        await t.test(g.name, async () => {
          const { playwright, neo } = await run(g.name)
          compareGolden(g, playwright, neo)
        })
      }
    },
  )
})

test("Neo's own click cases read the same through Playwright", {
  skip,
  timeout: 600_000,
}, async (t) => {
  await withBoth(neoCases, async (run) => {
    for (const c of neoCases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c.name)
        assert.equal(neo.outcome.status, 'ok', JSON.stringify(neo.outcome))
        assert.deepEqual(neo.result, playwright.result, 'what the script saw')
        assert.deepEqual(neo.probe, playwright.probe, 'what the page saw')
      })
    }
  })
})

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

/**
 * A golden case:
 * - `ok` resolves on both sides with the same URL, ending as recorded, and Neo's one record
 *   has delivery `done`;
 * - `strict`: `e.name` and the whole message equal;
 * - `hidden`: `e.name`, the first line and the call log equal;
 * - `covered`, `disabled`: `e.name`, the first line and the distinct call-log lines other than
 *   the retry waits equal, and each side's retry waits open the recorded backoff. How many
 *   attempts fit in the timeout depends on the machine's load, for Playwright as for Neo;
 *   neo-runtime's `click::` tests hold Neo's idle-browser log byte for byte under paused time.
 *
 * Every case: the same events reached both pages, and Playwright still reads as recorded.
 */
function compareGolden(g, playwright, neo) {
  const { outcome } = neo
  if (g.outcome === 'resolved') {
    assert.ok(
      playwright.result.value.endsWith(g.urlRelative),
      `Playwright as recorded: ${JSON.stringify(playwright.result)}`,
    )
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.equal(outcome.value, playwright.result.value, 'Neo vs Playwright')
    assert.deepEqual(
      outcome.calls.map((call) => call.delivery),
      ['done'],
    )
  } else {
    const recorded = g.error
    const theirs = playwright.result.error
    assert.ok(theirs, `Playwright threw: ${JSON.stringify(playwright.result)}`)
    assert.equal(outcome.status, 'error', JSON.stringify(outcome))
    const ours = outcome.error
    assert.equal(theirs.name, recorded.name, 'Playwright as recorded')
    assert.equal(
      firstLine(theirs.message),
      recorded.firstLine,
      'Playwright as recorded',
    )
    assert.equal(ours.name, theirs.name, 'Neo vs Playwright')
    assert.equal(
      firstLine(ours.message),
      firstLine(theirs.message),
      'Neo vs Playwright',
    )
    if (g.name === 'strict') {
      assert.equal(theirs.message, recorded.message, 'Playwright as recorded')
      assert.equal(ours.message, theirs.message, 'Neo vs Playwright')
    } else if (g.name === 'hidden') {
      assert.deepEqual(
        callLog(theirs.message),
        callLog(recorded.message),
        'Playwright as recorded',
      )
      assert.deepEqual(
        callLog(ours.message),
        callLog(theirs.message),
        'Neo vs Playwright',
      )
    } else {
      assert.deepEqual(
        keyLines(theirs.message),
        keyLines(recorded.message),
        'Playwright as recorded',
      )
      assertBackoff(theirs.message, recorded.message, 'Playwright')
      assert.deepEqual(
        keyLines(ours.message),
        keyLines(theirs.message),
        `Neo vs Playwright:\n${ours.message}`,
      )
      assertBackoff(ours.message, recorded.message, 'Neo')
    }
    assert.deepEqual(
      outcome.calls.map((call) => call.delivery),
      ['notSent'],
    )
  }
  assert.deepEqual(
    neo.probe,
    playwright.probe,
    'events the page saw, Neo vs Playwright',
  )
  assert.deepEqual(
    playwright.probe,
    g.page.events,
    'events the page saw, Playwright as recorded',
  )
}

function firstLine(message) {
  return message.split('\n')[0]
}

/** The lines after `Call log:` (Playwright's `formatCallLog`). */
function callLog(message) {
  const [, log = ''] = message.split('\nCall log:\n')
  return log.split('\n').filter(Boolean)
}

/**
 * A call-log line without nesting or fold count: `    2 × waiting for …` and
 * `  - waiting for …` both read `waiting for …`.
 */
function lineText(line) {
  return line.trimStart().replace(/^(- |\d+ × )/, '')
}

/** `_retryAction`'s wait between attempts (`waiting 20ms`), not a `waiting for …` line. */
function isRetryWait(text) {
  return /^waiting \d+ms$/.test(text)
}

/** The distinct call-log lines other than the retry waits. */
function keyLines(message) {
  const lines = callLog(message)
    .map(lineText)
    .filter((text) => !isRetryWait(text))
  return [...new Set(lines)].sort()
}

/** The retry waits must be a non-empty prefix of the recorded ones, in the folded log's order. */
function assertBackoff(message, recorded, side) {
  const waits = callLog(message).map(lineText).filter(isRetryWait)
  const backoff = callLog(recorded).map(lineText).filter(isRetryWait)
  assert.ok(
    waits.length > 0 && waits.every((wait, i) => wait === backoff[i]),
    `${side}'s retry waits ${JSON.stringify(waits)} must open the recorded backoff ${JSON.stringify(backoff)}:\n${message}`,
  )
}
