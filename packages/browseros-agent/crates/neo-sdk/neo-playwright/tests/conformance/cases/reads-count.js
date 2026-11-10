// count over reads.html and strict.html: matches now, without waiting, through shadow roots
// and hidden elements, and zero for nothing.
await page.goto(fixtures.url('reads.html'))
const counts = {
  buttons: await page.getByRole('button').count(),
  buttonElements: await page.locator('button').count(),
  listItems: await page.locator('li').count(),
  hiddenToo: await page.locator('p').count(),
  visibleOnly: await page.locator('p:visible').count(),
  hiddenRole: await page
    .getByRole('paragraph', { includeHidden: true })
    .count(),
  shadow: await page.locator('.in-shadow').count(),
  closedShadow: await page.getByText('Closed button').count(),
  none: await page.locator('#nothing').count(),
  text: await page.getByText('a', { exact: false }).count(),
}
await page.goto(fixtures.url('strict.html'))
counts.rows = await page.locator('.row').count()
counts.deletes = await page.getByRole('button', { name: 'Delete' }).count()
counts.rowTwo = await page
  .locator('.row')
  .filter({ hasText: 'Row two' })
  .getByRole('button')
  .count()
counts.nested = await page.getByRole('listitem').count()
counts.pageShortcutLocator = await page.locator('button').count()
return counts
