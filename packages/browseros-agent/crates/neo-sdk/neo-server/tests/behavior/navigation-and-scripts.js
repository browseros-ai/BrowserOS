// These retained product regressions cover navigation and main-world script seams beyond the
// core upstream selection. History uses the original no-BFCache fixture so load events fire.
// Assertions compare real values directly, without differential shims.
const response = await page.goto(`${origin}/history.html?name=First`)
expect(response.status()).toBe(200)
expect(response.ok()).toBe(true)
expect(await page.locator('#name').textContent()).toBe('First')
await page.goto(`${origin}/history.html?name=Second`)
await page.goBack()
expect(await page.locator('#name').textContent()).toBe('First')
await page.goForward()
expect(await page.locator('#name').textContent()).toBe('Second')
await page.waitForLoadState('domcontentloaded')
expect(
  await page.evaluate(({ a, b }) => [a + b, undefined, NaN, -0], {
    a: 2,
    b: 3,
  }),
).toEqual([5, undefined, NaN, -0])
let failed = false
try {
  await page.evaluate(() => {
    throw new TypeError('deliberate')
  })
} catch {
  failed = true
}
expect(failed).toBe(true)
expect(
  await page.locator('#name').evaluate((element) => element.textContent),
).toBe('Second')
await page.setContent(
  '<title>Content</title><iframe srcdoc="<p>Inside</p>"></iframe>',
)
await expect(page.frameLocator('iframe').locator('p')).toHaveText('Inside')
expect(
  await page
    .frames()[1]
    .evaluate(() => document.querySelector('p').textContent),
).toBe('Inside')
expect(await page.title()).toBe('Content')
const logs = []
page.on('console', (message) => logs.push(message.text()))
await page.evaluate(() => console.log('hosted-console-policy'))
await page.waitForTimeout(100)
expect(logs).toEqual([])
