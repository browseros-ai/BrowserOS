// Waiting reads: textContent waits for an element the page adds 400 ms after load, count does
// not wait, and a read of something that never comes times out with the op's own timeout.
await page.goto(fixtures.url('reads.html?late=400'))
const countBefore = await page.locator('#late').count()
const late = await harness.settle(page.locator('#late').textContent())
const never = await harness.settle(
  page.locator('#never').textContent({ timeout: 300 }),
)
page.setDefaultTimeout(250)
const defaultTimeout = await harness.settle(
  page.getByRole('button', { name: 'Never' }).textContent(),
)
return { countBefore, late, never, defaultTimeout }
