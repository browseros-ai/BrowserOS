// locator.toString() and locator.description() are synchronous and never touch the page: both
// engines run Playwright's own locator generators (`asLocatorDescription('javascript', …)`,
// `locatorCustomDescription`) over the locator's selector. Each builder form gives the same
// string in Neo as in Playwright; a `describe()` wins over the generated text and `description()`
// is `null` without one; a selector the generator cannot read comes back as is (`GN-025
// asLocatorDescription invalid input`). Cases after `LO-072`…`LO-078`
// (tests/page/locator-convenience.spec.ts) and `GN-001`…`GN-023`
// (tests/library/locator-generator.spec.ts), at 26137face.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('toString and description match Playwright for every builder form', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<!doctype html><title>toString</title><p id="p">p</p>',
    source: `
      const locators = {
        css: page.locator('div.item >> nth=2'),
        testId: page.getByTestId('He"llo'),
        testIdRegex: page.getByTestId(/He"llo/),
        text: page.getByText('Hello', { exact: true }),
        textRegex: page.getByText(/wor?ld/i),
        label: page.getByLabel('Name'),
        placeholder: page.getByPlaceholder('Search', { exact: true }),
        altText: page.getByAltText('Logo'),
        title: page.getByTitle(/Tip/),
        role: page.getByRole('button', { name: 'Submit' }),
        roleOptions: page.getByRole('checkbox', { checked: true, name: /Agree/, exact: false }),
        hasText: page.locator('li').filter({ hasText: 'Kale' }),
        hasNotText: page.locator('li').filter({ hasNotText: /Leek/ }),
        has: page.locator('section').filter({ has: page.getByRole('heading') }),
        hasNot: page.locator('section').filter({ hasNot: page.locator('img') }),
        visible: page.locator('button').filter({ visible: true }),
        firstLast: page.locator('li').first().last(),
        and: page.getByRole('button').and(page.getByTitle('Save')),
        or: page.getByRole('button').or(page.getByRole('link')),
        chain: page.locator('form').locator(page.getByLabel('Email')),
        frame: page.frameLocator('#login-iframe').getByRole('button', { name: 'Send' }),
        contentFrame: page.locator('iframe').contentFrame().locator('h1'),
        xpath: page.locator('//div[@id="p"]'),
        described: page.getByRole('button', { name: 'Submit' }).describe('Submit button'),
        quotes: page.locator('div').describe('Button with "quotes" and \\'apostrophes\\''),
        describedThenChained: page.locator('foo').describe('First').locator('button'),
        frameMain: page.mainFrame().locator('#p'),
        invalid: page.locator('internal:has-text=>>'),
      }
      const strings = Object.fromEntries(Object.entries(locators).map(([name, l]) => [name, l.toString()]))
      const templates = Object.fromEntries(Object.entries(locators).map(([name, l]) => [name, \`\${l}\`]))
      const descriptions = Object.fromEntries(Object.entries(locators).map(([name, l]) => [name, l.description()]))
      return { strings, sameAsTemplate: JSON.stringify(strings) === JSON.stringify(templates), descriptions }
    `,
    probe: 'document.title',
  })
  const { strings, sameAsTemplate, descriptions } = outcome.value
  assert.equal(sameAsTemplate, true)
  // Anchors from the pinned specs; every other form compared to Playwright by compareMethod.
  assert.equal(strings.testId, `getByTestId('He"llo')`)
  assert.equal(strings.role, `getByRole('button', { name: 'Submit' })`)
  assert.equal(strings.described, 'Submit button')
  assert.equal(descriptions.described, 'Submit button')
  assert.equal(descriptions.quotes, `Button with "quotes" and 'apostrophes'`)
  assert.equal(descriptions.describedThenChained, null)
  assert.equal(descriptions.role, null)
  assert.equal(strings.invalid, 'internal:has-text=>>')
  // No page call: toString and description are realm-only.
  assert.deepEqual(outcome.calls, [])
})
