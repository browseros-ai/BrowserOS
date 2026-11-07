// locator.contentFrame through the pinned Playwright and through Neo, on fresh tabs of
// tests/pages/frame-top.html: it returns a FrameLocator at once (not a promise) for the iframe
// its locator points at, by id, by getByTitle and by nth; it enters a cross-site,
// out-of-process frame, a same-origin chain two levels deep and the mixed tree (a 127.0.0.1
// frame inside the cross-site one); owner() leads back to the iframe; and a locator that is not
// an iframe, matches several iframes, or matches none fails the way frameLocator's does.
// Compared: what each call settled to (`frames.mjs` `settle`) and what every frame of each tab
// held afterwards, decoys included. Never wording, call logs, retry counts or timing.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, frames.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, failed } from './frames.mjs'
import { skip } from './harness.mjs'

const value = (v) => ({ value: v })

const cases = [
  {
    name: 'contentFrame enters the frame its locator points at',
    source: `const login = page.locator('#login-iframe').contentFrame()
    return {
      frameLocator: typeof login.then === 'undefined' && typeof login.getByLabel === 'function',
      filled: await settle(() => login.getByLabel('Email').fill('alice@example.com')),
      byTitle: await settle(() => page.getByTitle('Chain').contentFrame().locator('#who').textContent()),
      byNth: await settle(() => page.locator('.slot').nth(0).contentFrame().getByLabel('Email').inputValue()),
      nested: await settle(() => page.locator('#chain').contentFrame().locator('#chain-inner').contentFrame().locator('#who').textContent()),
      mixed: await settle(() => login.locator('#grandchild').contentFrame().getByLabel('Email').inputValue()),
      owner: await settle(() => page.locator('#same').contentFrame().owner().getAttribute('name')),
    }`,
    expected: {
      frameLocator: true,
      filled: 'undefined',
      byTitle: value('chain'),
      byNth: value('same@seed.test'),
      nested: value('chain-inner'),
      mixed: value('grandchild@seed.test'),
      owner: value('same'),
    },
    effect: (effect) => {
      const { cross } = assertDecoysUntouched(effect, ['cross'])
      assert.equal(cross.email, 'alice@example.com')
    },
    deliveries: { fill: ['done'] },
  },
  {
    // The 5 s timeouts only bound a regression: these fail before their first poll.
    name: 'not an iframe, several iframes, or none',
    source: `return {
      notAFrame: await settle(() => page.locator('#not-a-frame').contentFrame().locator('#who').textContent({ timeout: 5000 })),
      strict: await settle(() => page.locator('.slot').contentFrame().locator('#who').textContent({ timeout: 5000 })),
      missing: await settle(() => page.locator('#nope').contentFrame().locator('#who').textContent({ timeout: 500 })),
      missingCount: await settle(() => page.locator('#nope').contentFrame().locator('#who').count()),
    }`,
    expected: {
      notAFrame: failed('error'),
      strict: failed('strict'),
      missing: failed('timeout'),
      missingCount: value(0),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('contentFrame enters the same frames through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, cases)
})
