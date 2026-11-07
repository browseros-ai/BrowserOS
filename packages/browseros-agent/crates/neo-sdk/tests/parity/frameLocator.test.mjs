// frameLocator through the pinned Playwright and through Neo, each case on fresh tabs of
// tests/pages/frame-top.html: the owner's example (`page.frameLocator('#login-iframe')
// .getByLabel('Email').fill(…)`) into a cross-site, out-of-process frame; the Page, Frame (main
// and child) and Locator receivers; nested and mixed hops; the seven getBy* and first/last/nth;
// owner(); locator(Locator) from the same and from another frame; hops refused at once (not an
// iframe, inside a composite locator, several iframes for a strict call); a missing iframe; and
// frames reloaded, replaced or removed between calls. Compared: what each call settled to (its
// value, or its error's category and class, `frames.mjs` `settle`) and what every frame of each
// tab held afterwards, decoys included. Never wording, call logs, retry counts, timing or
// coordinates.
//
// Known, intended difference, asserted per engine and not counted as parity: Neo refuses
// `frameLocator()` with no selector (Playwright's `internal:control=any-frame`) with an `Error`
// (design §4.9), where Playwright reads from whichever frame matches.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, frames.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  FRAME_TREE,
  failed,
  pristine,
} from './frames.mjs'
import { skip } from './harness.mjs'

const RESOLVED = 'undefined'
const value = (v) => ({ value: v })

const cases = [
  {
    name: "the owner's example, and every receiver",
    source: `const login = page.frameLocator('#login-iframe')
    const filled = await settle(() => login.getByLabel('Email').fill('alice@example.com'))
    return {
      filled,
      page: await settle(() => page.frameLocator('#same').locator('#who').textContent()),
      mainFrame: await settle(() => page.mainFrame().frameLocator('#chain').locator('#who').textContent()),
      childFrame: await settle(() => page.frame('cross').frameLocator('#grandchild').locator('#who').textContent()),
      locator: await settle(() => page.locator('#cross-wrap').frameLocator('iframe').locator('#who').textContent()),
      nested: await settle(() => page.frameLocator('#chain').frameLocator('#chain-inner').locator('#who').textContent()),
      mixed: await settle(() => login.frameLocator('#grandchild').getByLabel('Email').inputValue()),
    }`,
    expected: {
      filled: RESOLVED,
      page: value('same'),
      mainFrame: value('chain'),
      childFrame: value('grandchild'),
      locator: value('cross'),
      nested: value('chain-inner'),
      mixed: value('grandchild@seed.test'),
    },
    effect: (effect) => {
      const { cross } = assertDecoysUntouched(effect, ['cross'])
      assert.equal(cross.email, 'alice@example.com')
    },
    deliveries: { fill: ['done'] },
  },
  {
    name: 'getBy* inside a frame, and first, last and nth',
    source: `const cross = page.frameLocator('#login-iframe')
    const slots = page.frameLocator('.slot')
    return {
      testId: await settle(() => cross.getByTestId('card').textContent()),
      altText: await settle(() => cross.getByAltText('Logo').count()),
      label: await settle(() => cross.getByLabel('Email').inputValue()),
      placeholder: await settle(() => cross.getByPlaceholder('Search').fill('found')),
      text: await settle(() => cross.getByText('cross two').textContent()),
      title: await settle(() => cross.getByTitle('Tip').textContent()),
      role: await settle(() => cross.getByRole('checkbox', { name: 'Agree' }).isChecked()),
      first: await settle(() => slots.first().locator('#who').textContent()),
      nth: await settle(() => slots.nth(1).locator('#who').textContent()),
      last: await settle(() => slots.last().getByLabel('Email').inputValue()),
      strict: await settle(() => slots.locator('#who').textContent({ timeout: 5000 })),
    }`,
    expected: {
      testId: value('card cross'),
      altText: value(1),
      label: value('cross@seed.test'),
      placeholder: RESOLVED,
      text: value('cross two'),
      title: value('tip cross'),
      role: value(false),
      first: value('same'),
      nth: value('cross'),
      last: value('chain@seed.test'),
      strict: failed('strict'),
    },
    effect: (effect) => {
      const { cross } = assertDecoysUntouched(effect, ['cross'])
      assert.equal(cross.search, 'found')
      assert.equal(cross.email, 'cross@seed.test')
    },
    deliveries: { fill: ['done'] },
  },
  {
    name: 'owner, and locator(Locator) from the same and from another frame',
    source: `const same = page.frameLocator('#same')
    return {
      ownerName: await settle(() => same.owner().getAttribute('name')),
      ownerCount: await settle(() => page.frameLocator('.slot').owner().count()),
      ownerOfNth: await settle(() => page.frameLocator('.slot').nth(1).owner().getAttribute('title')),
      sameFrame: await settle(() => same.locator(page.locator('#who')).textContent()),
      otherFrame: await settle(() => same.locator(page.frame('cross').locator('#who')).textContent()),
      composite: await settle(() => page.locator('p', { has: same.locator('#who') }).count()),
    }`,
    expected: {
      ownerName: value('same'),
      ownerCount: value(3),
      ownerOfNth: value('Login'),
      sameFrame: value('same'),
      otherFrame: failed('error'),
      composite: failed('error'),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
  {
    // The 5 s timeouts only bound a regression: the refused hops fail before their first poll.
    name: 'hops refused at once, and an iframe that is not there',
    source: `const notAFrame = page.frameLocator('#not-a-frame').locator('#who')
    const missing = page.frameLocator('#nope').locator('#who')
    return {
      notAFrame: await settle(() => notAFrame.textContent({ timeout: 5000 })),
      notAFrameVisible: await settle(() => notAFrame.isVisible()),
      nonStrictHop: await settle(() => page.frameLocator('.slot').locator('#who').count()),
      missing: await settle(() => missing.textContent({ timeout: 500 })),
      missingCount: await settle(() => missing.count()),
      missingVisible: await settle(() => missing.isVisible()),
    }`,
    expected: {
      notAFrame: failed('error'),
      notAFrameVisible: failed('error'),
      nonStrictHop: value(1),
      missing: failed('timeout'),
      missingCount: value(0),
      missingVisible: value(false),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
  {
    name: 'frameLocator() with no selector',
    source: `const anyFrame = page.frameLocator()
    return {
      read: await settle(() => anyFrame.getByText('grandchild two').textContent({ timeout: 5000 })),
      nth: await settle(() => anyFrame.first()),
    }`,
    expected: { read: value('grandchild two'), nth: failed('error') },
    expectedNeo: { read: failed('error'), nth: failed('error') },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
  {
    name: 'a frame reloaded or replaced is found again',
    source: `const same = page.frameLocator('#same')
    const cross = page.frameLocator('#login-iframe')
    await page.click('#reload-same')
    await same.getByText('gen 2').waitFor()
    const reloaded = await settle(() => same.getByLabel('Email').fill('after-reload'))
    await page.click('#replace-cross')
    await cross.getByText('gen 2').waitFor()
    const replaced = await settle(() => cross.getByLabel('Email').fill('after-replace'))
    return { reloaded, replaced }`,
    expected: { reloaded: RESOLVED, replaced: RESOLVED },
    effect: (effect) => {
      const { top, same, cross } = assertDecoysUntouched(effect, [
        'top',
        'same',
        'cross',
      ])
      assert.deepEqual(
        { gen: same.gen, email: same.email },
        { gen: 2, email: 'after-reload' },
      )
      assert.deepEqual(
        { gen: cross.gen, email: cross.email },
        { gen: 2, email: 'after-replace' },
      )
      assert.deepEqual(top.iframes, pristine('top').iframes)
    },
    deliveries: { fill: ['done', 'done'] },
  },
  {
    name: 'a removed frame times out, counts 0 and is never filled',
    source: `const same = page.frameLocator('#same')
    const before = await settle(() => same.locator('#who').textContent())
    await page.click('#remove-same')
    return {
      before,
      read: await settle(() => same.locator('#who').textContent({ timeout: 500 })),
      count: await settle(() => same.locator('#who').count()),
      fill: await settle(() => same.getByLabel('Email').fill('gone', { timeout: 500 })),
    }`,
    expected: {
      before: value('same'),
      read: failed('timeout'),
      count: value(0),
      fill: failed('timeout'),
    },
    effect: (effect) => {
      const frames = Object.keys(FRAME_TREE).filter((who) => who !== 'same')
      const { top } = assertDecoysUntouched(effect, ['top'], frames)
      assert.equal(top.email, 'top@seed.test')
      assert.deepEqual(top.iframes, [
        'login-iframe',
        'chain',
        'hidden-same',
        'hidden-cross',
      ])
    },
  },
]

test('frameLocator reaches the same frames through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await compareFrameCases(t, cases)
})
