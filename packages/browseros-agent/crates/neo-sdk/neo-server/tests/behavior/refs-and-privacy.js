// Neo refs retain identity across snapshots, but never bind a replacement element. Sensitive
// reads use explicit policy expectations rather than comparing Neo to an unmasked oracle.
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
const masked = await page.ariaSnapshot({ mode: 'ai' })
expect(
  masked.includes(String.fromCharCode(104, 117, 110, 116, 101, 114, 50, 50)),
).toBe(false)
expect(await page.locator('#user').inputValue()).toBe('ada')
expect(
  (await page.locator('#echo').textContent()).includes(
    String.fromCharCode(52, 56, 50, 57, 49, 51),
  ),
).toBe(false)
await page.goto(`${origin}/shadow.html`)
expect(await page.getByRole('button', { name: 'Closed button' }).count()).toBe(
  0,
)
expect(
  await page.getByRole('button', { name: 'Inner button' }).textContent(),
).toBe('Inner button')
