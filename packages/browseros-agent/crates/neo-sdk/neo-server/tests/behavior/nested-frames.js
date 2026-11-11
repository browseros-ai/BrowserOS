// Cross-site nested frames retain their own refs and route input to their own document. These
// expectations come from the fixture's declared tree and event journal, not captured Neo output.
await page.goto(`${origin}/nest/top.html`)
let snapshot = ''
for (
  let attempt = 0;
  attempt < 20 && !snapshot.includes('C4 button');
  attempt++
)
  snapshot = await page.ariaSnapshot({ mode: 'ai' })
const names = ['Top', 'A1', 'B2', 'B3', 'B1', 'C2', 'A3', 'C4']
expect(page.frames().length).toBe(8)
for (const name of names) {
  const button = new RegExp(`button "${name} button" \\[ref=([^\\]]+)\\]`).exec(
    snapshot,
  )?.[1]
  const field = new RegExp(`textbox "${name} field" \\[ref=([^\\]]+)\\]`).exec(
    snapshot,
  )?.[1]
  expect(typeof button).toBe('string')
  expect(typeof field).toBe('string')
  expect(await page.getByRef(button).textContent()).toBe(`${name} button`)
  await page.getByRef(field).fill(`${name} typed`)
}
for (const [titles, name] of [
  [['a1', 'b2', 'b3'], 'B3'],
  [['b1', 'c2', 'a3', 'c4'], 'C4'],
]) {
  let frame = page
  for (const title of titles)
    frame = frame.frameLocator(`iframe[title=${title}]`)
  const button = frame.getByRole('button', { name: `${name} button` })
  await button.hover()
  await page.waitForTimeout(200)
  await button.click()
  const state = JSON.parse(
    await frame.locator('html').getAttribute('data-state'),
  )
  expect(state.presses).toEqual([`${name} button`])
  expect(state.clicks).toEqual([`${name} button`])
  expect(state.values[`${name} field`]).toBe(`${name} typed`)
}
const oldRef = /button "C4 button" \[ref=([^\]]+)\]/.exec(snapshot)[1]
await page.evaluate(() => document.querySelector('iframe[title=b1]').remove())
let removed = false
try {
  await page.getByRef(oldRef).textContent({ timeout: 300 })
} catch {
  removed = true
}
expect(removed).toBe(true)
expect(
  await page.getByRole('button', { name: 'Top button' }).textContent(),
).toBe('Top button')
