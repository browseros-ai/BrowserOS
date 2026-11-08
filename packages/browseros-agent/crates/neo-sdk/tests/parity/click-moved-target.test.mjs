// locator.click through Neo and through real Playwright on one BrowserOS neo, for a target the
// page moves while it handles the press: the REAL bench's gocalendar-3/4/5 failure, where Neo's
// click returned ok on an MUI popover button that jumped 114 px between its pointerdown and its
// pointerup, so the release and the click landed on the popover's body and the editor never
// opened.
//
// moves-on-press.html is that popover: placed against the viewport's bottom from its size, and
// placed again whenever it handles a press. Content it gets while the tab is hidden waits for that
// next placement. Neo hides a tab between two `run` calls (its session's focus emulation ends with
// the run), so a two-call flow (open the popover, then click Edit) leaves Neo's popover needing a
// new place, which Edit's own mousedown applies; Playwright's tab never goes hidden. Playwright
// judges only a press's first event, so either engine on that page would report a click Edit
// never got. Neo also judges the release, so the press that landed off the moved button is
// blocked and pressed again where the button went, and the page ends as Playwright's does.
//
// Each case compares what each script saw (a value, or an error's kind) and what each page's own
// listeners saw. It never compares wording, call logs, coordinates or retry timing. Playwright
// connects only for its own half, so its focus emulation never keeps Neo's tab visible.
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
  connectCdp,
  connectReference,
  eventually,
  launchBrowser,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
  waitForUrl,
} from './harness.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const page = (file) => readFileSync(join(here, '../pages', file), 'utf8')
/** Fresh tabs the two-call regression runs on in each engine: enough to show it is no luck. */
const REGRESSION_TRIES = 20

const OPEN = `await page.getByRole('button', { name: 'Open' }).click()
await page.getByRole('dialog', { name: 'Details' }).waitFor()`
/** Reads whether the popover still waits for its new place, then clicks Edit. */
const EDIT = `const pending = await page.evaluate(() => window.__grown)
await page.getByRole('button', { name: 'Edit' }).click()
return { pending, edited: page.url().endsWith('#edit') }`

const cases = {
  normal: {
    html: page('moves-on-press.html'),
    steps: [`${OPEN}\n${EDIT}`],
    probe: 'window.__edits',
    returns: { value: { pending: false, edited: true } },
    page: 1,
  },
  covered: {
    html: page('covered.html'),
    steps: [
      "await page.getByRole('button', { name: 'Submit' }).click({ timeout: 1000 })",
    ],
    probe:
      "window.__events.some((event) => event.target === 'button' && event.type === 'click')",
    returns: { error: 'TimeoutError' },
    page: false,
  },
  menu: {
    html: page('covers-on-press.html'),
    steps: ["await page.getByRole('button', { name: 'Choose' }).click()"],
    probe: '[window.__chosen, window.__dismissed]',
    returns: { value: null },
    page: [1, 0],
  },
  'self-hiding': {
    html: page('covers-on-press.html'),
    steps: ["await page.getByRole('button', { name: 'Dismiss' }).click()"],
    probe: '[window.__chosen, window.__dismissed]',
    returns: { value: null },
    page: [0, 1],
  },
  'moved across the call boundary': {
    html: page('moves-on-press.html'),
    steps: [OPEN, EDIT],
    probe: 'window.__edits',
    page: 1,
  },
}

test('a click lands on its target through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await withBoth(async (run) => {
    for (const name of ['normal', 'covered', 'menu', 'self-hiding']) {
      await t.test(name, async () => {
        const c = cases[name]
        const { playwright, neo } = await run(name)
        assert.deepEqual(
          kind(playwright.results.at(-1)),
          c.returns,
          'Playwright',
        )
        assert.deepEqual(kind(neo.results.at(-1)), c.returns, 'Neo')
        assert.deepEqual(playwright.probe, c.page, "Playwright's page")
        assert.deepEqual(neo.probe, c.page, "Neo's page")
      })
    }
  })
})

test('a target that moves across the call boundary is clicked as Playwright clicks it', {
  skip,
  timeout: 900_000,
}, async () => {
  const c = cases['moved across the call boundary']
  await withBoth(async (run) => {
    for (let attempt = 1; attempt <= REGRESSION_TRIES; attempt++) {
      const { playwright, neo } = await run('moved across the call boundary')
      const tried = `try ${attempt} of ${REGRESSION_TRIES}`
      // Playwright's popover never waits for a new place; Neo's does, every time: the hide
      // between its calls happened before Edit's click.
      assert.deepEqual(
        playwright.results.map(kind),
        [{ value: null }, { value: { pending: false, edited: true } }],
        `Playwright, ${tried}`,
      )
      assert.deepEqual(
        neo.results.map(kind),
        [{ value: null }, { value: { pending: true, edited: true } }],
        `Neo, ${tried}`,
      )
      assert.equal(playwright.probe, c.page, `Playwright's page, ${tried}`)
      assert.equal(neo.probe, c.page, `Neo's page, ${tried}`)
      // One click record per step, each delivered once: Open's, then Edit's.
      assert.deepEqual(
        neo.outcomes.map((outcome) =>
          outcome.calls
            .filter((call) => call.method === 'click')
            .map((call) => call.delivery),
        ),
        [['done'], ['done']],
        `Neo's delivery, ${tried}`,
      )
    }
  })
})

/** A script's value as is (`undefined` as `null`), or its error's name, never its wording. */
function kind(result) {
  if (result.error) return { error: result.error.name }
  return { value: result.value ?? null }
}

/**
 * One headless browser, one fixture server serving every case's page at `/<name>.html`, the
 * test's own CDP client, and neo_run built. `body` gets `run(name)`, which runs that case's steps
 * on a fresh tab in Playwright, then on a fresh background tab in Neo, and returns both sides.
 */
async function withBoth(body) {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const pages = Object.fromEntries(
      Object.entries(cases).map(([name, c]) => [`/${slug(name)}.html`, c.html]),
    )
    const origin = await serveFixture(cleanups, { pages })
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-parity-scripts-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    await body(async (name) => {
      const c = cases[name]
      const url = `${origin}/${slug(name)}.html`
      return {
        playwright: await viaPlaywright(port, url, c),
        neo: await viaNeo(cdp, port, neoRun, scripts, url, c),
      }
    })
  } finally {
    await runCleanups(cleanups)
  }
}

function slug(name) {
  return name.replaceAll(' ', '-')
}

/**
 * Runs the case's steps one after another on one Playwright page, as an agent's consecutive
 * calls would, then reads the page. Playwright connects for this half only.
 */
async function viaPlaywright(port, url, c) {
  const playwright = await connectReference(port)
  try {
    const page = await playwright.contexts()[0].newPage()
    await page.goto(url)
    const AsyncFunction = (async () => {}).constructor
    const results = []
    for (const step of c.steps)
      results.push(
        await new AsyncFunction('page', step)(page).then(
          (value) => ({ value }),
          (error) => ({ error: { name: error.name, message: error.message } }),
        ),
      )
    const probe = await page.evaluate(c.probe)
    await page.close()
    return { results, probe }
  } finally {
    await playwright.close()
  }
}

/**
 * Runs each step as its own neo_run on one background tab, as consecutive `run` calls would.
 * Between two steps it waits until the page has seen itself hidden: Neo's hide between runs is
 * what the regression needs, so the test waits for it instead of hoping it came in time.
 */
async function viaNeo(cdp, port, neoRun, scripts, url, c) {
  const { targetId } = await cdp.send('Target.createTarget', {
    url,
    background: true,
  })
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  const read = async (expression) =>
    (
      await cdp.send(
        'Runtime.evaluate',
        { expression, returnByValue: true },
        sessionId,
      )
    ).result.value
  try {
    await waitForUrl(cdp, sessionId, url)
    const results = []
    const outcomes = []
    for (const [index, step] of c.steps.entries()) {
      if (index > 0)
        await eventually(
          async () => (await read('window.__hidden')) >= index,
          'Neo hid the tab after its run',
        )
      const scriptPath = join(scripts, `step-${index}.js`)
      writeFileSync(scriptPath, step)
      const neo = startNeo(neoRun, port, targetId, scriptPath)
      await neo.connected
      const outcome = await neo.outcome
      outcomes.push(outcome)
      results.push(
        outcome.status === 'ok'
          ? { value: outcome.value }
          : { error: outcome.error },
      )
    }
    return { results, outcomes, probe: await read(c.probe) }
  } finally {
    await cdp.send('Target.closeTarget', { targetId })
  }
}
