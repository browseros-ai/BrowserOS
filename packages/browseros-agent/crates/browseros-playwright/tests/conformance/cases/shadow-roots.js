// Shadow roots on shadow.html: CSS and role locators pierce open roots at any depth, slotted
// light-DOM text reads where it is slotted, XPath does not pierce, and a closed root stays
// closed.
await page.goto(fixtures.url('shadow.html'))
const read = (locator) =>
  harness.settle(
    locator.count().then(async (count) => ({
      count,
      text: count === 1 ? await locator.textContent() : null,
    })),
  )
return {
  innerCss: await read(page.locator('#inner button')),
  innerRole: await read(page.getByRole('button', { name: 'Inner button' })),
  innerLabel: await read(page.getByLabel('Inner field')),
  outerText: await read(page.getByText('Outer text')),
  slotted: await read(page.getByText('Slotted label')),
  xpath: await read(page.locator('xpath=//button')),
  closed: await read(page.getByRole('button', { name: 'Closed button' })),
  chain: await read(page.locator('#outer').locator('#inner').getByRole('button')),
  snapshot: await harness.settle(page.ariaSnapshot()),
}
