// locator.ariaSnapshot() in both modes: one element's subtree, a locator that waits for its
// element, and one that never matches.
await page.goto(fixtures.url('reads.html?late=400'))
return {
  list: await harness.settle(page.locator('#fruits').ariaSnapshot()),
  listAi: await harness.settle(
    page.locator('#fruits').ariaSnapshot({ mode: 'ai' }),
  ),
  nav: await harness.settle(page.getByRole('navigation').ariaSnapshot()),
  late: await harness.settle(page.locator('#late').ariaSnapshot()),
  missing: await harness.settle(
    page.locator('#never').ariaSnapshot({ timeout: 300 }),
  ),
  missingAi: await harness.settle(
    page.locator('#never').ariaSnapshot({ mode: 'ai', timeout: 300 }),
  ),
}
