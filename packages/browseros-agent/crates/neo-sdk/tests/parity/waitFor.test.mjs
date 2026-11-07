// locator.waitFor through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/wait-for.html: every state already met, met later by page timers, and never met
// (a timeout); calls refused at once (a strict mode violation, an invalid selector, an unknown
// state); and a wait that a click in the same script meets, which Neo's waits allow because they
// never take the tab's input. Compared: what each call settled to (`undefined` or its error's
// kind, `waits.mjs` `settle`) and what the page logged. Never wording, call logs, retry counts or
// timing.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, waits.mjs).

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  FRAME_TREE,
  failed,
  pristine,
} from './frames.mjs'
import { skip } from './harness.mjs'
import { withBoth } from './waits.mjs'

const html = readFileSync(
  new URL('../pages/wait-for.html', import.meta.url),
  'utf8',
)
const RESOLVED = 'undefined'
const TIMEOUT = { error: 'timeout' }

const cases = [
  {
    name: 'each state already met',
    source: `return {
      attached: await settle(page.locator('#hidden').waitFor({ state: 'attached' })),
      visible: await settle(page.locator('#shown').waitFor()),
      hidden: await settle(page.locator('#hidden').waitFor({ state: 'hidden' })),
      hiddenWithoutMatch: await settle(page.locator('#missing').waitFor({ state: 'hidden' })),
      detached: await settle(page.locator('#missing').waitFor({ state: 'detached' })),
    }`,
    expected: {
      attached: RESOLVED,
      visible: RESOLVED,
      hidden: RESOLVED,
      hiddenWithoutMatch: RESOLVED,
      detached: RESOLVED,
    },
  },
  {
    name: 'each state met later by page timers',
    source: `await page.click('#arm')
    const [attached, detached, visible, hidden] = await Promise.all([
      settle(page.locator('#late').waitFor({ state: 'attached' })),
      settle(page.locator('#doomed').waitFor({ state: 'detached' })),
      settle(page.locator('#reveal').waitFor({ state: 'visible' })),
      settle(page.locator('#fade').waitFor({ state: 'hidden' })),
    ])
    return { attached, detached, visible, hidden }`,
    probe: 'window.__log',
    expected: {
      attached: RESOLVED,
      detached: RESOLVED,
      visible: RESOLVED,
      hidden: RESOLVED,
    },
    expectedProbe: ['armed', 'changed'],
  },
  {
    name: 'each state never met times out',
    source: `return {
      visible: await settle(page.locator('#hidden').waitFor({ timeout: 300 })),
      attached: await settle(page.locator('#missing').waitFor({ state: 'attached', timeout: 300 })),
      detached: await settle(page.locator('#shown').waitFor({ state: 'detached', timeout: 300 })),
      hidden: await settle(page.locator('#shown').waitFor({ state: 'hidden', timeout: 300 })),
    }`,
    expected: {
      visible: TIMEOUT,
      attached: TIMEOUT,
      detached: TIMEOUT,
      hidden: TIMEOUT,
    },
  },
  {
    // The 5 s timeouts only bound a regression: each of these fails before its first poll.
    name: 'refused at once',
    source: `return {
      strict: await settle(page.locator('.many').waitFor({ timeout: 5000 })),
      strictHidden: await settle(page.locator('.many').waitFor({ state: 'hidden', timeout: 5000 })),
      selector: await settle(page.locator('p >> nonsense=1').waitFor({ timeout: 5000 })),
      state: await settle(page.locator('#shown').waitFor({ state: 'gone', timeout: 5000 })),
    }`,
    expected: {
      strict: { error: 'strict' },
      strictHidden: { error: 'strict' },
      selector: { error: 'error' },
      state: { error: 'error' },
    },
  },
  {
    name: 'met by a click in the same script',
    source: `const added = settle(page.locator('#added').waitFor())
    const clicked = await settle(page.click('#add'))
    return { clicked, added: await added }`,
    probe: 'window.__log',
    expected: { clicked: RESOLVED, added: RESOLVED },
    expectedProbe: ['added'],
  },
]

test('locator.waitFor settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await withBoth(t, { pages: { '/wait-for.html': html } }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run({ ...c, path: '/wait-for.html' })
        assert.deepEqual(playwright.result, { value: c.expected }, 'Playwright')
        assert.deepEqual(neo.result, playwright.result, 'Neo vs Playwright')
        assert.deepEqual(playwright.probe, c.expectedProbe, 'Playwright page')
        assert.deepEqual(neo.probe, playwright.probe, 'what the page saw')
      })
    }
  })
})

/**
 * locator.waitFor inside frames, on tabs of frame-top.html (frames.mjs, its `settle`): #arm makes
 * every frame add #late 500 ms later and the page add a new iframe, so each wait sees an element
 * appear inside a frame, or inside a frame that did not exist when it started. Where the iframe
 * is missing, `hidden` and `detached` hold at once and `attached` and `visible` time out; inside
 * a hidden iframe nothing is ever visible; and waits on a frame that is removed end when it goes.
 */
const frameCases = [
  {
    name: 'an element appears later inside each frame, and in a frame added later',
    source: `await page.click('#arm')
    const late = (frame, state) => settle(() => frame.locator('#late').waitFor({ state }))
    const login = page.frameLocator('#login-iframe')
    const chain = page.frameLocator('#chain')
    const [top, same, cross, grandchild, chainFrame, chainInner, hiddenSame, hiddenCross, lateFrame] =
      await Promise.all([
        late(page),
        late(page.frameLocator('#same')),
        late(login),
        late(login.frameLocator('#grandchild')),
        late(chain),
        late(chain.frameLocator('#chain-inner')),
        late(page.frameLocator('#hidden-same'), 'attached'),
        late(page.frameLocator('#hidden-cross'), 'attached'),
        settle(() => page.frameLocator('#late-frame').locator('#who').waitFor()),
      ])
    return { top, same, cross, grandchild, chainFrame, chainInner, hiddenSame, hiddenCross, lateFrame }`,
    expected: {
      top: RESOLVED,
      same: RESOLVED,
      cross: RESOLVED,
      grandchild: RESOLVED,
      chainFrame: RESOLVED,
      chainInner: RESOLVED,
      hiddenSame: RESOLVED,
      hiddenCross: RESOLVED,
      lateFrame: RESOLVED,
    },
    effect: (effect) => {
      const armed = Object.keys(FRAME_TREE)
      const touched = assertDecoysUntouched(effect, armed, [...armed, 'late'])
      for (const [who, facts] of Object.entries(touched))
        assert.equal(facts.late, true, `${who} added #late`)
      assert.deepEqual(touched.top.iframes, [
        ...pristine('top').iframes,
        'late-frame',
      ])
    },
  },
  {
    // The 5 s timeouts only bound a regression: each of these settles before its first poll.
    name: 'a missing iframe is hidden and detached at once; a hidden one shows nothing',
    source: `const missing = page.frameLocator('#nope').locator('#who')
    const inHidden = page.frameLocator('#hidden-same').locator('#send')
    return {
      hidden: await settle(() => missing.waitFor({ state: 'hidden', timeout: 5000 })),
      detached: await settle(() => missing.waitFor({ state: 'detached', timeout: 5000 })),
      attached: await settle(() => missing.waitFor({ state: 'attached', timeout: 300 })),
      visible: await settle(() => missing.waitFor({ timeout: 300 })),
      hiddenInHiddenFrame: await settle(() => inHidden.waitFor({ state: 'hidden', timeout: 5000 })),
      visibleInHiddenFrame: await settle(() => inHidden.waitFor({ timeout: 300 })),
    }`,
    expected: {
      hidden: RESOLVED,
      detached: RESOLVED,
      attached: failed('timeout'),
      visible: failed('timeout'),
      hiddenInHiddenFrame: RESOLVED,
      visibleInHiddenFrame: failed('timeout'),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
  {
    name: 'waits on a frame end when its iframe is removed',
    source: `const login = page.frameLocator('#login-iframe')
    // Both frame chains are resolved and warm before the waits start.
    await login.locator('#who').waitFor({ state: 'attached' })
    await login.frameLocator('#grandchild').locator('#send').waitFor()
    const detached = settle(() => login.locator('#who').waitFor({ state: 'detached', timeout: 5000 }))
    const hidden = settle(() => login.frameLocator('#grandchild').locator('#send').waitFor({ state: 'hidden', timeout: 5000 }))
    // The removal ends one task of the top page. Every poll of either wait starts in the top page,
    // where it finds #login-iframe, so while the task runs no poll can start into the
    // out-of-process frame, and one already in it has the task's second to finish; after it, the
    // owner is gone. A removal between a poll's two steps (a page.click lands anywhere) sends that
    // poll's next step into the closed session, and Playwright's wait rejects ("has been closed").
    await page.evaluate(() => {
      const end = Date.now() + 1000
      while (Date.now() < end) {}
      document.getElementById('remove-cross').click()
    })
    return { detached: await detached, hidden: await hidden }`,
    expected: { detached: RESOLVED, hidden: RESOLVED },
    effect: (effect) => {
      const frames = Object.keys(FRAME_TREE).filter(
        (who) => who !== 'cross' && who !== 'grandchild',
      )
      const { top } = assertDecoysUntouched(effect, ['top'], frames)
      assert.deepEqual(top.iframes, [
        'same',
        'chain',
        'hidden-same',
        'hidden-cross',
      ])
    },
  },
]

test('locator.waitFor inside frames settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
