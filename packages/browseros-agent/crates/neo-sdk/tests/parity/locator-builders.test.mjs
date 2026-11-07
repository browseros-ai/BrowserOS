// The locator builders through Neo and Playwright on one fixture (tests/pages/locator-builders.html):
// what each builder MATCHES (the count, and each match's id, or tag and text), on each receiver
// Playwright has it on: getByRole/Text/Label/Placeholder/AltText/Title/TestId on Page, Frame,
// Locator and FrameLocator; locator (a string and a Locator), filter (hasText, hasNotText, has,
// hasNot, visible), first/last/nth, and/or, within, visible, describe, frameLocator on every
// receiver, Locator.page, FrameLocator.owner and FrameLocator.first/last/nth; then a strict mode
// violation and an invalid selector. Matches are read with count() and evaluateAll in document
// order, so both engines are compared on the same elements; a few key matches are also pinned.
// Each scoped or optioned case has a contrasting element outside its scope or option (the
// fixture's "outside" elements, a second heading, two iframe holders), so a builder that loses
// its receiver, its frame or an option matches more and fails the comparison.
//
// From Playwright's specs at the pin (26137face): tests/page/selectors-get-by.spec.ts 'SE-076
// getByText should work', tests/page/locator-query.spec.ts 'LO-177 should support
// locator.within' and tests/page/locator-frame.spec.ts 'LO-110 frameLocator.owner should work'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/locator-builders.html', import.meta.url),
  'utf8',
)

/** What a locator matches, and the frames loaded first. */
const PRELUDE = `
const matches = async (locator) => ({
  count: await locator.count(),
  ids: await locator.evaluateAll((elements) =>
    elements.map((e) => e.id || e.tagName.toLowerCase() + ':' + e.textContent.trim())),
})
await page.frameLocator('#first-frame').frameLocator('#deep').locator('p').waitFor({ state: 'attached' })
await page.frameLocator('#second-frame').locator('#second-button').waitFor({ state: 'attached' })
const all = async (cases) => {
  const out = {}
  for (const [name, locator] of Object.entries(cases)) out[name] = await matches(locator)
  return out
}
`

const ids = (value, name) => value[name].ids

test('every getBy builder matches the same elements on Page, Frame, Locator and FrameLocator', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${PRELUDE}
      const frame = page.frame('first-frame')
      const main = page.mainFrame()
      const form = page.locator('#form')
      const inner = page.frameLocator('#first-frame')
      return await all({
        'page.getByRole': page.getByRole('button'),
        'page.getByRole name': page.getByRole('button', { name: 'Buy' }),
        'page.getByRole listitem': page.getByRole('listitem'),
        'page.getByRole level': page.getByRole('heading', { level: 1 }),
        'page.getByRole disabled': page.getByRole('button', { disabled: true }),
        'page.getByText': page.getByText('Apple'),
        'page.getByText exact': page.getByText('Buy', { exact: true }),
        'page.getByText regex': page.getByText(/^Ban/),
        'page.getByLabel': page.getByLabel('Name'),
        'page.getByLabel nested': page.getByLabel('Email'),
        'page.getByPlaceholder': page.getByPlaceholder('Your name'),
        'page.getByAltText': page.getByAltText('Company logo'),
        'page.getByTitle': page.getByTitle('Helpful hint'),
        'page.getByTestId': page.getByTestId('special'),
        'page.locator': page.locator('li'),
        'frame.locator': frame.locator('#inner-text'),
        'mainFrame.getByRole': main.getByRole('heading'),
        'frame.getByRole': frame.getByRole('button'),
        'frame.getByText': frame.getByText('Inner text'),
        'frame.getByLabel': frame.getByLabel('Query'),
        'frame.getByPlaceholder': frame.getByPlaceholder('Search'),
        'frame.getByAltText': frame.getByAltText('Inner logo'),
        'frame.getByTitle': frame.getByTitle('Inner hint'),
        'frame.getByTestId': frame.getByTestId('inner'),
        'locator.getByRole': form.getByRole('button'),
        'locator.getByText': form.getByText('Test id'),
        'locator.getByLabel': form.getByLabel('Name'),
        'locator.getByPlaceholder': form.getByPlaceholder('you@example.com'),
        'locator.getByAltText': form.getByAltText('Company logo'),
        'locator.getByTitle': form.getByTitle('Helpful hint'),
        'locator.getByTestId': form.getByTestId('special'),
        'frameLocator.getByRole': inner.getByRole('button'),
        'frameLocator.getByText': inner.getByText('Inner text'),
        'frameLocator.getByLabel': inner.getByLabel('Query'),
        'frameLocator.getByPlaceholder': inner.getByPlaceholder('Search'),
        'frameLocator.getByAltText': inner.getByAltText('Inner logo'),
        'frameLocator.getByTitle': inner.getByTitle('Inner hint'),
        'frameLocator.getByTestId': inner.getByTestId('inner'),
      })
    `,
    probe: 'document.title',
  })
  const value = outcome.value
  // A hidden element is no role match; getByText still counts it.
  assert.deepEqual(ids(value, 'page.getByRole name'), [
    'buy-apple',
    'buy-banana',
  ])
  assert.deepEqual(ids(value, 'page.getByRole listitem'), [
    'apple',
    'banana',
    'date',
  ])
  assert.deepEqual(ids(value, 'page.getByText exact'), [
    'buy-apple',
    'buy-banana',
    'buy-cherry',
  ])
  assert.deepEqual(ids(value, 'page.getByLabel nested'), ['email'])
  assert.deepEqual(ids(value, 'locator.getByLabel'), ['name'])
  // Each also matches outside the form, so a Locator builder that loses its scope fails.
  for (const [name, expected] of [
    ['locator.getByText', ['special']],
    ['locator.getByPlaceholder', ['email']],
    ['locator.getByAltText', ['logo']],
    ['locator.getByTitle', ['hint']],
    ['locator.getByTestId', ['special']],
  ])
    assert.deepEqual(ids(value, name), expected, name)
  assert.deepEqual(ids(value, 'page.getByRole level'), ['heading'])
  assert.deepEqual(ids(value, 'frameLocator.getByTestId'), ['inner-special'])
  assert.deepEqual(ids(value, 'page.locator'), [
    'apple',
    'banana',
    'cherry',
    'date',
  ])
  for (const [name, { count }] of Object.entries(value))
    assert.ok(count > 0, `${name} matches something`)
})

test('the composing builders match the same elements', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${PRELUDE}
      const items = page.locator('li')
      const cases = await all({
        'locator string': page.locator('#form').locator('input'),
        'locator Locator': page.locator('#form').locator(page.locator('button')),
        'filter hasText': items.filter({ hasText: 'Banana' }),
        'filter hasNotText': items.filter({ hasNotText: 'Banana' }),
        'filter has': items.filter({ has: page.getByRole('button') }),
        'filter hasNot': items.filter({ hasNot: page.getByRole('button') }),
        'filter visible': items.filter({ visible: true }),
        'filter hidden': items.filter({ visible: false }),
        first: items.first(),
        last: items.last(),
        nth: items.nth(2),
        'nth negative': items.nth(-1),
        and: page.getByRole('button').and(page.locator('#submit')),
        or: page.locator('#apple').or(page.locator('#date')),
        within: page.getByRole('cell').nth(1).within(page.getByRole('row')),
        visible: items.visible(),
        describe: page.locator('#submit').describe('the submit button'),
      })
      return { ...cases, page: page.locator('#submit').page() === page }
    `,
    probe: 'document.title',
  })
  const value = outcome.value
  assert.deepEqual(ids(value, 'filter has'), ['apple', 'banana'])
  assert.deepEqual(ids(value, 'filter hidden'), ['cherry'])
  assert.deepEqual(ids(value, 'within'), ['a2', 'b2'])
  assert.deepEqual(ids(value, 'nth'), ['cherry'])
  assert.equal(value.page, true)
})

test('frameLocator on every receiver, FrameLocator.first/last/nth/owner, and the errors', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${PRELUDE}
      const frames = page.frameLocator('iframe')
      const cases = await all({
        'page.frameLocator': page.frameLocator('#first-frame').locator('#inner-text'),
        'frame.frameLocator': page.mainFrame().frameLocator('#second-frame').locator('p'),
        'frame.frameLocator child': page.frame('first-frame').frameLocator('#deep').locator('p'),
        'locator.frameLocator': page.locator('#second-holder').frameLocator('iframe').getByRole('button'),
        'frameLocator.frameLocator': page.frameLocator('#first-frame').frameLocator('#deep').locator('p'),
        'frameLocator.locator': page.frameLocator('#second-frame').locator('button'),
        'frameLocator.locator Locator': page.frameLocator('#second-frame').locator(page.locator('#second-button')),
        'frameLocator.first': frames.first().locator('button'),
        'frameLocator.last': frames.last().locator('button'),
        'frameLocator.nth': frames.nth(1).locator('button'),
        'frameLocator.owner': page.frameLocator('#second-frame').owner(),
        'frameLocator.owner nested': page.frameLocator('#first-frame').frameLocator('#deep').owner(),
      })
      const strict = await page.getByRole('button', { name: 'Buy' }).click({ timeout: 1000 })
        .then(() => 'resolved', errorCategory)
      const invalid = await page.locator('#form >> unknown-engine=x').count().then(() => 'resolved', errorCategory)
      return { ...cases, strict, invalid }
    `,
    probe: 'document.title',
  })
  const value = outcome.value
  assert.deepEqual(ids(value, 'frameLocator.frameLocator'), ['p:Deep'])
  assert.deepEqual(ids(value, 'frameLocator.first'), ['inner-button'])
  assert.deepEqual(ids(value, 'frameLocator.last'), ['second-button'])
  assert.deepEqual(ids(value, 'frameLocator.owner'), ['second-frame'])
  // A frame or scope the builder lost would reach the other iframe, or none.
  assert.deepEqual(ids(value, 'frame.frameLocator child'), ['p:Deep'])
  assert.deepEqual(ids(value, 'locator.frameLocator'), ['second-button'])
  assert.deepEqual(ids(value, 'frameLocator.owner nested'), ['deep'])
  assert.deepEqual(ids(value, 'frameLocator.locator Locator'), [
    'second-button',
  ])
  assert.equal(value.strict, 'Error:strict')
  assert.equal(value.invalid, 'Error')
})
