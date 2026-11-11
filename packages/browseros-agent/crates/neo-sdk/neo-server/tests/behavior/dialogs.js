// Runtime callbacks retain their closure and settle native dialogs through the event pump.
// Values are the page's actual answers; no transport error can stand in for an assertion.
await page.goto(`${origin}/dialogs.html`)
const alert = page.waitForEvent('dialog')
await page.locator('#alert').click()
const first = await alert
expect(first.type()).toBe('alert')
expect(first.message()).toBe('Hello')
await first.accept()
const confirm = page.waitForEvent('dialog')
await page.locator('#confirm').click()
const second = await confirm
expect(second.type()).toBe('confirm')
await second.dismiss()
const prompt = page.waitForEvent('dialog')
await page.locator('#prompt').click()
const third = await prompt
expect(third.defaultValue()).toBe('Ada')
await third.accept('Grace')
await expect
  .poll(() => page.evaluate(() => window.__answers))
  .toEqual([null, false, 'Grace'])
const waited = await page.waitForFunction(() => window.__answers.length === 3)
expect(await waited.jsonValue()).toBe(true)
