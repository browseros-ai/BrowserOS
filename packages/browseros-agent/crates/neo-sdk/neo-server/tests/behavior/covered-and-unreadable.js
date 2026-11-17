// A covered nested-frame target receives no input. A hung sibling must not prevent a partial
// accessibility snapshot of the other frames; this guards routing and bounded frame traversal.
await page.goto(`${origin}/covered-frame.html`)
const child = page
  .frameLocator('iframe[title=L1]')
  .frameLocator('iframe[title=L2]')
  .getByRole('button', { name: 'Child button' })
let covered = false
try {
  await child.click({ timeout: 1500 })
} catch {
  covered = true
}
expect(covered).toBe(true)
const l2 = page.frames().find((frame) => frame.url().includes('name=L2'))
const events = JSON.parse(await l2.locator('html').getAttribute('data-events'))
expect(
  events.filter((event) =>
    ['mousedown', 'mouseup', 'click'].includes(event.type),
  ),
).toEqual([])
await page.goto(`${origin}/nest/top.html?hang`)
await page.waitForTimeout(2000)
const start = Date.now()
const snapshot = await page.ariaSnapshot({ mode: 'ai', timeout: 8000 })
expect(Date.now() - start).toBeLessThan(4000)
expect(snapshot.includes('Top button')).toBe(true)
expect(snapshot.includes('C4 button')).toBe(true)
