// locator.all() is Playwright's client body over `count()` and `nth(i)`: one locator per match
// at the time of the call, in document order, each a locator of the same page (hidden matches
// included, as count counts them). No match is [] at once, under a 1 ms default timeout; an
// invalid selector rejects. Cases after `LO-126 locator.all should work`
// (tests/page/locator-list.spec.ts at 26137face).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('locator.all returns one nth locator per match', {
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
      const veg = await page.locator('#veg > li').all()
      const texts = []
      // all() itself takes no timeout; reading each match does.
      for (const item of veg) texts.push(await item.textContent({ timeout: 5000 }))
      const missing = await page.locator('.missing').all()
      const chained = await page.locator('#fruits').locator('.fruit').filter({ hasText: 'a' }).all()
      const shapes = [...veg, ...chained].map(l => typeof l.click === 'function' && l.page() === page)
      let invalid = 'resolved'
      try { await page.locator('#veg >>').all() } catch (error) { invalid = errorCategory(error) }
      return { texts, missing: missing.length, chained: chained.length, shapes, invalid }
    `,
    probe: `[...document.querySelectorAll('#veg > li')].map(e => e.textContent)`,
  })
  assert.deepEqual(outcome.value, {
    texts: ['Kale', 'Carrot', 'Leek'],
    missing: 0,
    chained: 3,
    shapes: Array(6).fill(true),
    invalid: 'Error',
  })
  // One count per all(); the reads come from the nth locators.
  assert.equal(outcome.calls.filter((c) => c.method === 'queryCount').length, 4)
})

test('review: all returns live nth locators in their original frame', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const child = `<ul><li>A</li><li>B</li><li>C</li></ul>
    <button id="remove" onclick="document.querySelector('li').remove()">Remove first</button>`
  const outcome = await compareMethod(t, {
    html: `<iframe id="child"></iframe><script>
      document.querySelector('iframe').srcdoc = ${JSON.stringify(child)}
    </script>`,
    source: `
      const frame = page.frameLocator('#child')
      const items = await frame.locator('li').all()
      await frame.locator('#remove').click()
      const first = await items[0].textContent()
      const second = await items[1].textContent()
      let removed = 'resolved'
      try { await items[2].textContent({ timeout: 500 }) }
      catch (error) { removed = errorCategory(error) }
      return { count: items.length, first, second, removed }
    `,
    probe:
      '[...document.querySelector("iframe").contentDocument.querySelectorAll("li")].map(e => e.textContent)',
  })
  assert.deepEqual(outcome.value, {
    count: 3,
    first: 'B',
    second: 'C',
    removed: 'TimeoutError',
  })
})
