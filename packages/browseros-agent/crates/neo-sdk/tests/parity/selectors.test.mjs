// playwright.selectors through Neo and through Playwright on one BrowserOS neo:
// `setTestIdAttribute` changes the attribute every later `getByTestId` selects by, on the page
// and through a locator, and the old attribute no longer matches.
//
// Playwright case (pin 26137face): `tests/page/selectors-get-by.spec.ts`, which sets
// `playwright.selectors.setTestIdAttribute('data-my-custom-testid')` before its getByTestId
// cases.
//
// Deliberate difference, asserted on Neo: `selectors.register` refuses (a custom engine is the
// script's code run in every page, which Neo's page JS guard does not allow); Playwright's is
// not run here, since a registered engine stays in this test process.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/test-id.html':
    '<title>test id</title><p data-neo-id="target">custom</p><p data-testid="legacy">legacy</p>',
}

const source = `const before = await page.getByTestId('legacy').count()
playwright.selectors.setTestIdAttribute('data-neo-id')
try {
  return {
    before,
    custom: await page.getByTestId('target').count(),
    text: await page.locator('body').getByTestId('target').textContent(),
    old: await page.getByTestId('legacy').count(),
  }
} finally {
  playwright.selectors.setTestIdAttribute('data-testid')
}`

test('playwright.selectors: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/test-id.html', source))
    assert.deepEqual(reference, {
      before: 1,
      custom: 1,
      text: 'custom',
      old: 0,
    })
    assert.deepEqual(
      outcomeValue(await neo('/test-id.html', source)),
      reference,
    )
    const register = await neo(
      '/test-id.html',
      `return await playwright.selectors.register('tag', { content: '({ query: () => null, queryAll: () => [] })' }).then(() => 'registered', errorCategory)`,
    )
    assert.equal(outcomeValue(register), 'Error')
  })
})
