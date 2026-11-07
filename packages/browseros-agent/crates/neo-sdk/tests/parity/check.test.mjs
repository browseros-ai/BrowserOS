// Check compares real clicks, radio/ARIA targets, an existing state and trial behavior.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import {
  assertDecoysUntouched,
  compareFrameCases,
  failed,
  value,
} from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('check returns undefined and sets checked state on every receiver', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/check-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [await page.check('#unchecked', null) === undefined,
        await page.mainFrame().check('#radio', null) === undefined,
        await page.locator('#aria').check() === undefined,
        await page.locator('#checked').check() === undefined,
        await page.locator('#trial').check({trial:true}) === undefined,
        await page.locator('#replaced').check() === undefined]
      if (voids.length !== 6 || !voids.every(Boolean)) throw new Error('check must return undefined')
      let wrong = 'resolved'
      try { await page.locator('#plain').check() } catch (error) { wrong = errorCategory(error) }
      if (wrong !== 'error') throw new Error('wrong element must reject')
      let timeout = 'resolved'
      try { await page.locator('#covered').check({timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'timeout') throw new Error('covered check must time out')
      return { voids, wrong, timeout }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(
    outcome.calls.map((c) => c.delivery),
    [
      'done',
      'done',
      'done',
      'notSent',
      'notSent',
      'done',
      'notSent',
      'notSent',
    ],
  )
})

/** check inside frames, on tabs of frame-top.html (frames.mjs): each frame's own Agree box and
 * no decoy, the cross-site frame's 127.0.0.1 child included; a box in a hidden iframe stays
 * unchecked. */
const frameCases = [
  {
    name: "check ticks one frame's Agree box and no decoy",
    source: `const login = page.frameLocator('#login-iframe')
    return {
      same: await settle(() => page.frameLocator('#same').getByLabel('Agree').check()),
      cross: await settle(() => login.getByRole('checkbox', { name: 'Agree' }).check()),
      frame: await settle(() => page.frame('chain-inner').check('#agree')),
      readBack: await settle(() => login.locator('#agree').isChecked()),
      hiddenFrame: await settle(() => page.frameLocator('#hidden-same').locator('#agree').check({ timeout: 500 })),
    }`,
    expected: {
      same: 'undefined',
      cross: 'undefined',
      frame: 'undefined',
      readBack: value(true),
      hiddenFrame: failed('timeout'),
    },
    effect: (effect) => {
      const touched = assertDecoysUntouched(effect, [
        'same',
        'cross',
        'chain-inner',
      ])
      for (const [who, facts] of Object.entries(touched))
        assert.equal(facts.agree, true, `${who}'s Agree`)
    },
    deliveries: { check: ['done', 'done', 'done', 'notSent'] },
  },
]

test('check inside frames ticks the same box through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
