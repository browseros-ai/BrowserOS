// The dialog crosses Chromium's native event boundary. A real user gesture arms beforeunload;
// accepting it must settle the close event and mark the same Page object closed.
await page.goto(`${origin}/dialogs.html?beforeunload`)
await page.mouse.click(2, 2)
const dialog = page.waitForEvent('dialog', { timeout: 3000 })
const closed = new Promise((resolve) => page.once('close', resolve))
await page.close({ runBeforeUnload: true })
const prompted = await dialog
expect(prompted.type()).toBe('beforeunload')
await prompted.accept()
await closed
expect(page.isClosed()).toBe(true)
