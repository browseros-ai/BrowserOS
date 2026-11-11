// Neo refs retain identity across snapshots, but never bind a replacement element; a sensitive
// field and the page text around it read as the page holds them.
await page.setContent(
  '<button>Original</button><iframe srcdoc="<button>Child</button>"></iframe>',
)
let snapshot = ''
for (
  let attempt = 0;
  attempt < 20 && !snapshot.includes('button "Child"');
  attempt++
)
  snapshot = await page.ariaSnapshot({ mode: 'ai' })
const main = /button "Original" \[ref=((?:f\d+)?e\d+)\]/.exec(snapshot)?.[1]
const child = /button "Child" \[ref=(f\d+e\d+)\]/.exec(snapshot)?.[1]
expect(typeof main).toBe('string')
expect(typeof child).toBe('string')
expect(await page.getByRef(child).textContent()).toBe('Child')
expect(await page.getByRef(main).textContent()).toBe('Original')
await page.locator('button').ariaSnapshot()
await page.ariaSnapshot()
await page.ariaSnapshot({ mode: 'ai' })
expect(await page.getByRef(main).textContent()).toBe('Original')
await page.evaluate(() => {
  document.querySelector('button').outerHTML = '<button>Replacement</button>'
})
let detached
try {
  await page.getByRef(main).click({ timeout: 300 })
} catch (error) {
  detached = error
}
expect(Boolean(detached)).toBe(true)
expect(
  await page.getByRole('button', { name: 'Replacement' }).textContent(),
).toBe('Replacement')
await page.goto(`${origin}/sensitive.html`)
expect(await page.locator('#password').inputValue()).toBe(
  String.fromCharCode(104, 117, 110, 116, 101, 114, 50, 50),
)
expect(await page.locator('#user').inputValue()).toBe('ada')
expect(await page.locator('#echo').textContent()).toBe(
  `Confirm your code ${String.fromCharCode(52, 56, 50, 57, 49, 51)}`,
)
await page.goto(`${origin}/shadow.html`)
expect(await page.getByRole('button', { name: 'Closed button' }).count()).toBe(
  0,
)
expect(
  await page.getByRole('button', { name: 'Inner button' }).textContent(),
).toBe('Inner button')
