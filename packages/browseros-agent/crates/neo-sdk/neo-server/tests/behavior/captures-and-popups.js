// Format and privacy contracts belong to Neo's behavior suite. Pixel compatibility is checked
// independently by the common screenshot programs, against the native BrowserOS oracle.
await page.goto(`${origin}/child.html`)
const png = await page.screenshot()
expect(Array.from(png.slice(0, 8))).toEqual([137, 80, 78, 71, 13, 10, 26, 10])
const jpeg = await page.screenshot({ type: 'jpeg', quality: 50 })
expect(Array.from(jpeg.slice(0, 3))).toEqual([255, 216, 255])
const pdf = await page.pdf()
expect(String.fromCharCode(...pdf.slice(0, 5))).toBe('%PDF-')
const opened = page.waitForEvent('popup', { timeout: 5000 })
await page.evaluate((url) => {
  window.testPopup = window.open(url)
}, `${origin}/child.html?name=Popup`)
const popup = await opened
await popup.waitForLoadState()
await page.evaluate(() => {
  const input = document.createElement('input')
  input.type = 'password'
  input.id = 'private'
  input.value = String.fromCharCode(112).repeat(17)
  document.body.append(input)
})
expect(
  await popup.evaluate(() => opener.document.querySelector('#private').value),
).toBe(String.fromCharCode(112).repeat(17))
await popup.evaluate(() => {
  const input = document.createElement('input')
  input.type = 'password'
  input.id = 'private'
  input.value = String.fromCharCode(113).repeat(19)
  document.body.append(input)
})
expect(
  await page.evaluate(
    () => window.testPopup.document.querySelector('#private').value,
  ),
).toBe(String.fromCharCode(113).repeat(19))
expect((await popup.opener()) === page).toBe(true)
await popup.close()
