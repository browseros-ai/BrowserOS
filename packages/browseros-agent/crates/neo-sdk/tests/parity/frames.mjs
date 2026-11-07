// The runner the frame parity files share (frameLocator, contentFrame and frames.test.mjs, and
// the frame cases of the method files), on the shared harness: one BrowserOS neo, the fixture
// server serving tests/pages/frame-top.html and frame-child.html, the pinned Playwright over
// connectOverCDP, and neo_run. Each case runs its script on two fresh tabs of frame-top.html,
// Playwright's first, then Neo's (opened with the test's own CDP client, as a user's tab), and
// returns what each script returned and what every frame of each tab held afterwards: the case's
// `probe` evaluated in each frame through Playwright's `frame.evaluate`, out-of-process frames
// included, keyed by the frame's `who` (frame-child.html).
//
// Every script gets `settle` first, so both engines report a call the same way: `'undefined'`
// for a call that resolved `undefined`, `{ value }` for any other value, or the error's category
// (timeout, strict, closed or error) and its class name, never its wording. It takes a function,
// so a call that throws before it returns a promise is caught too.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  browserBinary,
  buildNeoRun,
  eventually,
  findPage,
  launchReference,
  playwrightVersion,
  referenceHalf,
  runCleanups,
  serveFixture,
  startNeo,
  waitForUrl,
} from './harness.mjs'

const page = (file) =>
  readFileSync(new URL(`../pages/${file}`, import.meta.url), 'utf8')

/** Every page a frame case loads; frame-child.html answers for any `?who=`. */
export const FRAME_PAGES = {
  '/frame-top.html': page('frame-top.html'),
  '/frame-child.html': page('frame-child.html'),
}

/** The frames frame-top.html loads, by `who`; `cross`, `grandchild` and `hidden-cross` are out
 * of process. */
export const FRAME_TREE = {
  top: null,
  same: 'top',
  cross: 'top',
  grandchild: 'cross',
  chain: 'top',
  'chain-inner': 'chain',
  'hidden-same': 'top',
  'hidden-cross': 'top',
}

/** What each frame records (frame-child.html's `window.__facts()`). */
export const FACTS = 'window.__facts()'

const SETTLE = `const settle = async (call) => {
  try {
    const value = await call()
    return value === undefined ? 'undefined' : { value }
  } catch (error) {
    return {
      error:
        error.name === 'TimeoutError' ? 'timeout'
        : /strict mode violation/.test(error.message) ? 'strict'
        : /has been closed/.test(error.message) ? 'closed'
        : 'error',
      name: error.name,
    }
  }
}
`

/** `settle`'s report of a call that resolved to `v`. */
export const value = (v) => ({ value: v })

/** `settle`'s report of a call that rejected with a plain `Error` of `category`. */
export const failed = (category) => ({
  error: category,
  name: category === 'timeout' ? 'TimeoutError' : 'Error',
})

/**
 * What `window.__facts()` reads in a frame no call has touched, for the frame `who` (and, for
 * the top page, the iframes it holds when nothing was added, replaced or removed).
 */
export function pristine(who) {
  const facts = {
    who,
    gen: 1,
    email: `${who}@seed.test`,
    search: '',
    agree: false,
    size: 's',
    late: false,
    events: [],
  }
  if (who === 'top')
    facts.iframes = [
      'same',
      'login-iframe',
      'chain',
      'hidden-same',
      'hidden-cross',
    ]
  return facts
}

/**
 * Asserts that `effect` holds exactly the frames in `frames`, and that each one outside
 * `touched` reads as `pristine`: a call routed to the wrong frame shows up as a decoy that
 * changed. Returns the touched frames' facts.
 */
export function assertDecoysUntouched(
  effect,
  touched,
  frames = Object.keys(FRAME_TREE),
) {
  assert.deepEqual(
    Object.keys(effect).sort(),
    [...frames].sort(),
    'the frames the page holds',
  )
  for (const who of frames) {
    if (!touched.includes(who))
      assert.deepEqual(effect[who], pristine(who), `decoy frame ${who}`)
  }
  return Object.fromEntries(touched.map((who) => [who, effect[who]]))
}

/**
 * What every frame of Playwright's `page` holds: `probe` evaluated in each frame, keyed by its
 * `window.__who`. Frames without one (none of the fixture's) are skipped; two frames with one
 * `who` fail the test.
 */
export async function frameEffects(page, probe = FACTS) {
  const effects = {}
  for (const frame of page.frames()) {
    const who = await frame.evaluate(() => window.__who)
    if (who === undefined) continue
    assert.ok(!(who in effects), `two frames are both ${who}`)
    effects[who] = await frame.evaluate(probe)
  }
  return effects
}

/**
 * When set, `compareFrameCases` runs only Playwright's half: the reference's values and decoys,
 * for checking a case before Neo serves what it calls. Never set in CI.
 */
const referenceOnly = Boolean(process.env.NEO_PARITY_REFERENCE_ONLY)

/**
 * Runs each case as a subtest of `t` on both engines (`withFrames`) and checks it:
 * - Playwright's value is `c.expected` (an entry that is a function asserts Playwright's value
 *   for its key instead, for values too long to spell out), and `c.effect` accepts Playwright's
 *   frames (it checks the decoys, `assertDecoysUntouched`);
 * - Neo's value is Playwright's, or `c.expectedNeo` where the design makes Neo differ;
 * - every frame of Neo's tab holds what the same frame of Playwright's does, or, where the
 *   design makes Neo leave the page differently, what `c.neoEffect` accepts;
 * - Neo's delivery record for each method in `c.deliveries` (calls in order).
 * Value keys in `c.perEngine` differ by design and are left out of both comparisons;
 * `c.checkPlaywright(playwright)` and `c.checkNeo(neo, playwright)` assert them, and anything
 * else, per engine.
 */
export async function compareFrameCases(t, cases, { slowMs = 0 } = {}) {
  await withFrames(t, { slowMs }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(c)
        const reference = shared(playwright.result, c.perEngine)
        assertExpected(reference, c.expected)
        c.effect?.(playwright.effect)
        await c.checkPlaywright?.(playwright)
        if (referenceOnly) return
        assert.deepEqual(
          shared(neo.result, c.perEngine),
          c.expectedNeo === undefined ? reference : { value: c.expectedNeo },
          c.expectedNeo === undefined ? 'Neo vs Playwright' : 'Neo, per design',
        )
        if (c.neoEffect) c.neoEffect(neo.effect)
        else
          assert.deepEqual(
            neo.effect,
            playwright.effect,
            'what every frame saw',
          )
        for (const [method, deliveries] of Object.entries(c.deliveries ?? {}))
          assert.deepEqual(
            neo.outcome.calls
              .filter((call) => call.method === method)
              .map((call) => call.delivery),
            deliveries,
            `Neo's ${method} delivery record`,
          )
        await c.checkNeo?.(neo, playwright)
      })
    }
  })
}

/** Asserts Playwright's `result` against `expected`, whose function entries assert their key. */
function assertExpected(result, expected) {
  const checked = Object.entries(
    expected !== null && typeof expected === 'object' ? expected : {},
  ).filter(([, check]) => typeof check === 'function')
  const keys = checked.map(([key]) => key)
  assert.deepEqual(
    shared(result, keys),
    shared({ value: expected }, keys),
    'Playwright',
  )
  for (const [key, check] of checked) check(result.value[key])
}

/** `result` without the value keys in `perEngine`. */
function shared(result, perEngine = []) {
  if (!('value' in result) || perEngine.length === 0) return result
  const value = { ...result.value }
  for (const key of perEngine) delete value[key]
  return { value }
}

/**
 * Launches the browser, serves `FRAME_PAGES` (`/slow` answers after `slowMs`) and connects both
 * engines, then calls `body` with
 * `run(c)`, which runs case `c` (its `source`, an optional `probe`) on both engines and returns
 * each side's `result`, per-frame `effect` and `iframes` (the out-of-process frames of its tab
 * as CDP listed them after the run), plus Neo's `outcome`. With `NEO_PARITY_REFERENCE_ONLY`,
 * Neo's side is `undefined`.
 */
export async function withFrames(t, { slowMs = 0 }, body) {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  if (referenceOnly) t.diagnostic('NEO_PARITY_REFERENCE_ONLY: Playwright only')
  const neoRun = referenceOnly ? undefined : buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages: FRAME_PAGES, slowMs })
    const { playwright, cdp, ...browser } = await launchReference(
      cleanups,
      browserBinary,
      { cdp: true },
    )
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-frame-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const url = `${origin}/frame-top.html`
    let next = 1
    await body(async (c) => ({
      playwright: await viaPlaywright(cdp, context, url, c),
      neo: referenceOnly
        ? undefined
        : await viaNeo(
            { cdp, context, port: browser.port, neoRun, scripts },
            next++,
            url,
            c,
          ),
    }))
  } finally {
    await runCleanups(cleanups)
  }
}

/**
 * The `iframe` targets (out-of-process frames) under the tab `targetId`, each as its URL's path
 * and query with the host kept and the port dropped (`localhost/frame-child.html?who=cross…`).
 */
async function iframeTargets(cdp, targetId) {
  const { targetInfos } = await cdp.send('Target.getTargets')
  const frames = targetInfos.filter((info) => info.type === 'iframe')
  const under = new Set([targetId])
  let grew = true
  while (grew) {
    grew = false
    for (const frame of frames) {
      if (under.has(frame.parentFrameId) && !under.has(frame.targetId)) {
        under.add(frame.targetId)
        grew = true
      }
    }
  }
  return frames
    .filter((frame) => frame.targetId !== targetId && under.has(frame.targetId))
    .map((frame) => frame.url.replace(/^http:\/\/([^:/]+):\d+/, '$1'))
    .sort()
}

/**
 * The viewport both tabs load frame-top.html in. At the browser's own size (516×475) the
 * cross-site frames start below the fold, and a click into an out-of-process frame that has just
 * scrolled into view can reach its parent's <iframe> element instead (seen once in about ten
 * Playwright runs, which still reported the click done). At this size they are on screen from
 * load; #chain-inner, a same-process frame, still starts below the fold.
 */
const VIEWPORT = {
  width: 800,
  height: 900,
  deviceScaleFactor: 1,
  mobile: false,
}

/** Runs the case as Playwright does, with `page` and `context` in scope (`referenceHalf`). */
async function viaPlaywright(cdp, context, url, c) {
  return await referenceHalf(async () => {
    const page = await context.newPage()
    try {
      // Kept attached to the end: the override lasts as long as the session that set it.
      const emulation = await context.newCDPSession(page)
      await emulation.send('Emulation.setDeviceMetricsOverride', VIEWPORT)
      await page.goto(url)
      const AsyncFunction = (async () => {}).constructor
      const result = await new AsyncFunction(
        'page',
        'context',
        SETTLE + c.source,
      )(page, context).then(
        (value) => ({ value }),
        (error) => ({ error: { name: error.name, message: error.message } }),
      )
      const session = await context.newCDPSession(page)
      const { targetInfo } = await session.send('Target.getTargetInfo')
      await session.detach()
      return {
        result,
        effect: await frameEffects(page, c.probe),
        iframes: await iframeTargets(cdp, targetInfo.targetId),
      }
    } finally {
      await page.close()
    }
  })
}

/**
 * Runs the case through neo_run on a tab of its own, opened as a user's tab would be (at
 * `VIEWPORT`), once the page and every frame in it have loaded. Playwright, attached to every
 * tab, reads its frames afterwards; the CDP target listing runs only after the run, so it does
 * not create out-of-process frame hosts Neo would have to create itself.
 */
async function viaNeo({ cdp, context, port, neoRun, scripts }, index, url, c) {
  const { targetId } = await cdp.send('Target.createTarget', {
    url: 'about:blank',
  })
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  try {
    // This session stays attached until the tab closes, so the override lasts the whole run.
    await cdp.send('Emulation.setDeviceMetricsOverride', VIEWPORT, sessionId)
    await cdp.send('Page.navigate', { url }, sessionId)
    await waitForUrl(cdp, sessionId, url)
    await eventually(
      async () =>
        (
          await cdp.send(
            'Runtime.evaluate',
            { expression: 'document.readyState', returnByValue: true },
            sessionId,
          )
        ).result.value === 'complete',
      `${url} loaded`,
    )
    const scriptPath = join(scripts, `frame-case-${index}.js`)
    writeFileSync(scriptPath, SETTLE + c.source)
    const neo = startNeo(neoRun, port, targetId, scriptPath)
    await neo.connected
    const outcome = await neo.outcome
    const result =
      outcome.status === 'ok'
        ? { value: outcome.value }
        : { error: outcome.error }
    const effect = await frameEffects(
      await findPage(context, targetId),
      c.probe,
    )
    return {
      outcome,
      result,
      effect,
      iframes: await iframeTargets(cdp, targetId),
    }
  } finally {
    await cdp.send('Target.closeTarget', { targetId })
  }
}
