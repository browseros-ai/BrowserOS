// allTextContents is one non-strict read of every match's textContent, in document order and
// untouched: nested markup flattened, whitespace kept, hidden text included, an empty element ''.
// No match is [] at once. It takes no timeout, so a 1 ms default timeout must not touch it.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

const FRUITS = ['Apple', '  Banana\n    split  ', '', 'Cherry', 'Date']

test('allTextContents returns the textContent of every match, at once', {
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
        fruits: await page.locator('.fruit').allTextContents(),
        missing: await page.locator('.missing').allTextContents(),
        chained: await page.locator('#veg').locator('li').filter({ hasText: 'e' }).allTextContents(),
      }
      if (!Object.values(lists).every(list => Array.isArray(list) && list.every(text => typeof text === 'string')))
        throw new Error('allTextContents must return string arrays')
      const expected = { fruits: ${JSON.stringify(FRUITS)}, missing: [], chained: ['Kale', 'Leek'] }
      if (JSON.stringify(lists) !== JSON.stringify(expected)) throw new Error('lists: ' + JSON.stringify(lists))
      let invalid = 'resolved'
      try { await page.locator('.fruit >>').allTextContents() } catch (error) { invalid = errorCategory(error) }
      if (invalid !== 'Error') throw new Error('an invalid selector must reject')
      return { lists, invalid }
    `,
    // The DOM's own textContent, which Playwright's page function returns as is.
    probe: `['.fruit', '#veg > li'].map(s => [...document.querySelectorAll(s)].map(e => e.textContent))`,
  })
  assert.ok(outcome.calls.every((c) => c.delivery === undefined))
})

/** allTextContents inside frames, on tabs of frame-top.html (frames.mjs): each frame names
 * itself in its items, hidden ones included; a missing iframe reads [] at once. */
const frameCases = [
  {
    name: 'allTextContents reads the items of the frame it names',
    source: `page.setDefaultTimeout(1)
    return {
      cross: await settle(() => page.frameLocator('#login-iframe').locator('.item').allTextContents()),
      frame: await settle(() => page.frame('grandchild').locator('.item').allTextContents()),
      missingFrame: await settle(() => page.frameLocator('#nope').locator('.item').allTextContents()),
    }`,
    expected: {
      cross: value(['cross one', 'cross two', 'cross hidden']),
      frame: value(['grandchild one', 'grandchild two', 'grandchild hidden']),
      missingFrame: value([]),
    },
    effect: (effect) => assertDecoysUntouched(effect, []),
  },
]

test('allTextContents inside frames reads the same texts through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})

test('review: allTextContents keeps SVG text and empty strings', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<span class="item">HTML</span><svg class="item"><text>SVG</text></svg><span class="item"></span>',
    source: `
      const values = await page.locator('.item').allTextContents()
      return { values, types: values.map(value => typeof value) }
    `,
    probe: '[...document.querySelectorAll(".item")].map(e => e.textContent)',
  })
  assert.deepEqual(outcome.value, {
    values: ['HTML', 'SVG', ''],
    types: ['string', 'string', 'string'],
  })
})
