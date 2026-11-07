// allInnerTexts is one non-strict read of every match's rendered `innerText`, in document order
// (Playwright's `$$eval(selector, ee => ee.map(e => e.innerText))`): whitespace collapsed as the
// page renders it, a display:none item's own text, an empty element ''. No match is [] at once.
// It takes no timeout, so a 1 ms default timeout must not touch it; an invalid selector rejects.
// Cases after `LO-070 allInnerTexts should work` (tests/page/locator-convenience.spec.ts at
// 26137face).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('allInnerTexts returns the innerText of every match, at once', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-lists.html', import.meta.url),
      'utf8',
    ),
    source: `
      page.setDefaultTimeout(1)
      const lists = {
        fruits: await page.locator('.fruit').allInnerTexts(),
        missing: await page.locator('.missing').allInnerTexts(),
        chained: await page.locator('#veg').locator('li').filter({ hasText: 'e' }).allInnerTexts(),
      }
      if (!Object.values(lists).every(list => Array.isArray(list) && list.every(text => typeof text === 'string')))
        throw new Error('allInnerTexts must return string arrays')
      let invalid = 'resolved'
      try { await page.locator('.fruit >>').allInnerTexts() } catch (error) { invalid = errorCategory(error) }
      return { lists, invalid }
    `,
    // The page's own innerText of each list, which Playwright's page function returns as is.
    probe: `['.fruit', '#veg > li'].map(s => [...document.querySelectorAll(s)].map(e => e.innerText))`,
  })
  assert.deepEqual(outcome.value, {
    lists: {
      fruits: ['Apple', 'Banana split', '', 'Cherry', 'Date'],
      missing: [],
      chained: ['Kale', 'Leek'],
    },
    invalid: 'Error',
  })
  assert.ok(outcome.calls.every((c) => c.delivery === undefined))
})

/** allInnerTexts inside frames, on tabs of frame-top.html (frames.mjs): each frame names
 * itself in its items; the hidden item keeps its own text; a missing iframe reads [] at once. */
const frameCases = [
  {
    name: 'allInnerTexts reads the items of the frame it names',
    source: `page.setDefaultTimeout(1)
    return {
      cross: await settle(() => page.frameLocator('#login-iframe').locator('.item').allInnerTexts()),
      frame: await settle(() => page.frame('grandchild').locator('.item').allInnerTexts()),
      missingFrame: await settle(() => page.frameLocator('#nope').locator('.item').allInnerTexts()),
    }`,
    expected: {
      cross: value(['cross one', 'cross two', 'cross hidden']),
      frame: value(['grandchild one', 'grandchild two', 'grandchild hidden']),
      missingFrame: value([]),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('allInnerTexts inside frames reads the same texts through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
