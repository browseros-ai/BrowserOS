// Hosted infrastructure lists all tabs; a refusal still belongs to its trace operation even
// when the agent catches it. Javascript navigation must fail before changing the page.
const before = context.pages().length
const second = await context.newPage()
expect(context.pages().includes(second)).toBe(true)
expect(context.pages().length).toBe(before + 1)
expect(second.context() === context).toBe(true)
await second.close()
expect(context.pages().length).toBe(before)
expect(second.isClosed()).toBe(true)
let error
try {
  await page.route('**/*', () => {})
} catch (caught) {
  error = caught
}
await page.title()
expect(error.code).toBe('method_unavailable')
expect(error.owner).toBe('script')
expect(typeof error.operationId).toBe('string')
await page.setContent('<title>unchanged</title>')
for (const url of [
  "javascript:document.title='changed'",
  " JavaScript:document.title='changed'",
]) {
  let refused = false
  try {
    await page.goto(url)
  } catch {
    refused = true
  }
  expect(refused).toBe(true)
}
expect(await page.title()).toBe('unchanged')
const opened = page.waitForEvent('popup', { timeout: 5000 })
await page.evaluate((url) => {
  window.open(url)
}, `${origin}/child.html?name=Popup`)
const popup = await opened
await popup.waitForLoadState()
expect((await popup.opener()) === page).toBe(true)
expect(context.pages().includes(popup)).toBe(true)
await popup.close()
expect(popup.isClosed()).toBe(true)
