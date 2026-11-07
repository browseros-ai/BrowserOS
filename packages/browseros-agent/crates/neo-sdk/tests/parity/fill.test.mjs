// Fill's Page, Frame and Locator receivers share one behavior check on fresh pages.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, failed } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('fill returns undefined and uses ordinary input events', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.fill('#text', 'Page', null) === undefined,
        await page.mainFrame().fill('#area', 'Frame', null) === undefined,
        await page.getByLabel('Name').fill('Locator') === undefined,
        await page.locator('#editable').fill('Editable') === undefined,
        await page.locator('#date').fill('2026-11-06') === undefined,
      ]
      if (voids.length !== 5 || !voids.every(Boolean)) throw new Error('fill must return undefined')
      let invalid = 'resolved'
      try { await page.locator('#text').fill(123) } catch (error) { invalid = errorCategory(error) }
      if (invalid !== 'error') throw new Error('invalid fill value must reject')
      let timeout = 'resolved'
      try { await page.locator('#disabled').fill('x', {timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'timeout') throw new Error('disabled fill must time out')
      return { voids, invalid, timeout }
    `,
    probe: 'window.__facts()',
  })
  const delivered = outcome.calls.filter((c) => c.result.kind === 'ok')
  assert.equal(delivered.length, 5)
  assert.ok(delivered.every((c) => c.delivery === 'done'))
})

/**
 * Fill inside frames, on tabs of frame-top.html (frames.mjs): through a frame locator, a nested
 * one into the 127.0.0.1 frame inside the cross-site one, and a child Frame; the parent's and
 * every other frame's Email keep their values, the nested frame's parent included. An input in
 * a hidden iframe is never filled.
 */
const frameCases = [
  {
    name: "fill types into one frame's Email and no decoy",
    source: `const login = page.frameLocator('#login-iframe')
    return {
      same: await settle(() => page.frameLocator('#same').getByLabel('Email').fill('same@new.test')),
      grandchild: await settle(() => login.frameLocator('#grandchild').getByLabel('Email').fill('grandchild@new.test')),
      frame: await settle(() => page.frame('chain-inner').fill('#email', 'chain-inner@new.test')),
      hiddenFrame: await settle(() => page.frameLocator('#hidden-same').getByLabel('Email').fill('never', { timeout: 500 })),
    }`,
    expected: {
      same: 'undefined',
      grandchild: 'undefined',
      frame: 'undefined',
      hiddenFrame: failed('timeout'),
    },
    effect: (effect) => {
      const touched = assertDecoysUntouched(effect, [
        'same',
        'grandchild',
        'chain-inner',
      ])
      for (const [who, facts] of Object.entries(touched))
        assert.equal(facts.email, `${who}@new.test`, `${who}'s Email`)
    },
    deliveries: { fill: ['done', 'done', 'done', 'notSent'] },
  },
]

test('fill inside frames fills the same input through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
