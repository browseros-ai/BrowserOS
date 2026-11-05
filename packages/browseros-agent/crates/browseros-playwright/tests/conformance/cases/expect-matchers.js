// expect's retrying matchers as scripts write them: passing locator and page matchers, `.not`,
// one that holds only once the page adds #late (400 ms after load), and failing ones that end
// at their timeout, a `.not` one included. Each failure keeps its error's name and first line.
await page.goto(fixtures.url('reads.html?late=400'))
const passing = {
  visible: await harness.settle(expect(page.locator('#save')).toBeVisible()),
  notVisible: await harness.settle(expect(page.locator('#hidden')).not.toBeVisible()),
  text: await harness.settle(expect(page.locator('#save')).toHaveText('Save')),
  containText: await harness.settle(
    expect(page.getByTestId('greeting')).toContainText('world'),
  ),
  count: await harness.settle(expect(page.locator('#fruits li')).toHaveCount(3)),
  value: await harness.settle(expect(page.locator('#email')).toHaveValue('ada@example.com')),
  attribute: await harness.settle(
    expect(page.getByText('Titled paragraph')).toHaveAttribute('title', 'Tooltip'),
  ),
  enabled: await harness.settle(expect(page.locator('#save')).toBeEnabled()),
  late: await harness.settle(expect(page.locator('#late')).toHaveText('Late text')),
  title: await harness.settle(expect(page).toHaveTitle('Reads')),
  url: await harness.settle(expect(page).toHaveURL(/reads\.html/)),
}
const failing = {
  text: await harness.settle(
    expect(page.locator('#save')).toHaveText('Send', { timeout: 300 }),
  ),
  missing: await harness.settle(
    expect(page.locator('#never')).toBeVisible({ timeout: 300 }),
  ),
  notVisible: await harness.settle(
    expect(page.locator('#save')).not.toBeVisible({ timeout: 300 }),
  ),
  count: await harness.settle(
    expect(page.locator('#fruits li')).toHaveCount(2, { timeout: 300 }),
  ),
  title: await harness.settle(expect(page).toHaveTitle('Writes', { timeout: 300 })),
}
return { passing, failing }
