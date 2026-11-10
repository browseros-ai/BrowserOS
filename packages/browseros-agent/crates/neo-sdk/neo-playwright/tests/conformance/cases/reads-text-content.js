// textContent through every kind of locator on reads.html: CSS, roles and names, text,
// labels, placeholders, alt text, titles, test ids, chaining, filters, positions, hidden
// elements and an open shadow root.
await page.goto(fixtures.url('reads.html'))
const reads = {
  css: page.locator('#save'),
  role: page.getByRole('button', { name: 'Save' }),
  ariaLabel: page.getByRole('button', { name: 'Cancel order' }),
  roleExact: page.getByRole('button', { name: 'Cancel order', exact: true }),
  heading: page.getByRole('heading', { level: 1 }),
  link: page.getByRole('navigation', { name: 'Main' }).getByRole('link'),
  text: page.getByText('Titled paragraph'),
  textSubstring: page.getByText('Hello,'),
  textRegex: page.getByText(/cherry/i),
  label: page.getByLabel('Email'),
  labelFor: page.getByLabel('Age'),
  placeholder: page.getByPlaceholder('Search'),
  alt: page.getByAltText('Logo picture'),
  title: page.getByTitle('Tooltip'),
  testId: page.getByTestId('greeting'),
  chained: page.locator('#fruits').locator('li').filter({ hasText: 'Banana' }),
  hasChild: page.locator('li').filter({ has: page.locator('b') }),
  hasNotText: page.locator('li').filter({ hasNotText: /a/ }).first(),
  nth: page.locator('#fruits li').nth(1),
  last: page.locator('#fruits li').last(),
  and: page.getByRole('button').and(page.locator('#cancel')),
  or: page.locator('#nothing').or(page.locator('#save')),
  hidden: page.locator('#hidden'),
  invisible: page.locator('#invisible'),
  shadow: page.locator('.in-shadow'),
  shadowRole: page.getByRole('button', { name: 'Shadow button' }),
  xpath: page.locator('xpath=//ul/li[1]'),
  pageShortcut: null,
}
const results = {}
for (const [name, locator] of Object.entries(reads))
  if (locator !== null)
    results[name] = await harness.settle(locator.textContent())
results.pageShortcut = await harness.settle(page.textContent('#save'))
return results
