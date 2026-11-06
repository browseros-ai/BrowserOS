// Async function body for Runtime::run: `page` is supplied by neo-runtime.
// Live Amazon.in scenario; requires an empty cart. Leaves one pack in the cart.
// Observed through BrowserOS neo; this file has not yet run through neo-runtime.

const productName =
  'Sensodyne Toothpaste Fresh Mint | Daily Sensitivity Relief Tooth Paste Pack of 1 - 75gm | Dentist Recommended | Everyday Use Paste, Fresh Breath, Fights Tooth Decay, Maintain Healthy Gums'
const resultName = productName.replace(/^Sensodyne /, '')

page.setDefaultTimeout(15_000)
page.setDefaultNavigationTimeout(30_000)
await page.goto('https://www.amazon.in/')

const initialSnapshot = await page.ariaSnapshot()
if (!initialSnapshot.includes('link "0 items in cart"')) {
  throw new Error('This scenario requires an empty Amazon.in cart.')
}

const search = page.getByRole('searchbox', { name: 'Search Amazon.in' })
await search.fill('sensodyne toothpaste single pack')
await search.press('Enter')

const product = page.getByRole('listitem').filter({
  has: page.getByRole('link', { name: resultName, exact: true }),
})
await product.getByRole('heading', { name: 'Sensodyne', exact: true }).textContent()
await product.getByRole('button', { name: 'Add to cart', exact: true }).click()
await page.getByRole('link', { name: 'Go to Cart', exact: true }).click()

const cart = page.getByRole('list', { name: 'Shopping Cart', exact: true })
const title = await cart
  .getByRole('heading', { name: productName, exact: true })
  .textContent()
if (title?.trim() !== productName) {
  throw new Error('The cart does not contain the expected single-pack toothpaste.')
}

const cartSnapshot = await page.ariaSnapshot()
if (
  !cartSnapshot.includes(
    `button "Increase quantity by one, Quantity is 1, ${productName}"`,
  )
) {
  throw new Error('Expected quantity 1 for the Sensodyne pack in the cart.')
}

return { product: title.trim(), quantity: 1, cartUrl: page.url() }
