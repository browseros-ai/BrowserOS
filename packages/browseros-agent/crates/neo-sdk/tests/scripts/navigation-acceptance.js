// The navigation and page-reading acceptance script: one run opens a page, reads it, clicks
// with the existing locator API (by a snapshot ref and by role), reads the change, follows a
// link to a second document and reads that. neo-runtime's tests/acceptance_e2e.rs runs it on
// a real BrowserOS neo against its fixture pages `/start` and `/next`.
const here = page.url()
const origin = here.slice(0, here.indexOf('/', 'http://'.length))

const navigated = await page.goto(`${origin}/start`)
const title = await page.title()
const snapshot = await page.ariaSnapshot({ mode: 'ai' })
const ref = /- button "Increment" \[ref=(e\d+)\]/.exec(snapshot)?.[1]
if (!ref) throw new Error(`no ref for the Increment button in:\n${snapshot}`)

const before = await page.locator('#status').textContent()
await page.locator(`aria-ref=${ref}`).click()
const afterRefClick = await page.locator('#status').textContent()
await page.getByRole('button', { name: 'Increment' }).click()
const afterRoleClick = await page.getByText(/Clicked \d+ times/).textContent()

await page.getByRole('link', { name: 'Next page' }).click()
const nextTitle = await page.title()
const nextText = await page.locator('#message').textContent()
const nextSnapshot = await page.ariaSnapshot()
const staleRef = await page
  .locator(`aria-ref=${ref}`)
  .click({ timeout: 500 })
  .then(
    () => 'clicked',
    (error) => error.name,
  )

return {
  navigated,
  title,
  ref,
  snapshot,
  before,
  afterRefClick,
  afterRoleClick,
  nextUrl: page.url().slice(origin.length),
  nextTitle,
  nextText,
  nextSnapshot,
  staleRef,
}
