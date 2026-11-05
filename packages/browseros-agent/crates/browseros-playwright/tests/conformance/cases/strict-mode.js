// Strict mode on strict.html: a read that matches three elements fails at once, and each way
// of narrowing it to one element reads that element.
await page.goto(fixtures.url('strict.html'))
const deletes = page.getByRole('button', { name: 'Delete' })
return {
  textContent: await harness.settle(deletes.textContent()),
  css: await harness.settle(page.locator('.row button').textContent()),
  pageShortcut: await harness.settle(page.textContent('.row button')),
  snapshot: await harness.settle(deletes.ariaSnapshot()),
  first: await harness.settle(deletes.first().textContent()),
  nth: await harness.settle(
    page.locator('.row').nth(1).locator('span').textContent(),
  ),
  last: await harness.settle(page.locator('.row span').last().textContent()),
  filtered: await harness.settle(
    page.locator('.row').filter({ hasText: 'Row three' }).locator('span').textContent(),
  ),
  nestedItems: await harness.settle(page.getByRole('listitem').textContent()),
  count: await deletes.count(),
}
