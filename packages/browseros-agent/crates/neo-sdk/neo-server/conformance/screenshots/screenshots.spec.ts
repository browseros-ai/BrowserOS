// These explicit screenshot checks complement the Node-only upstream screenshot spec.
// They share the common program runner; pixels are compared outside the JS realms, without
// teaching either browser implementation what the other's output should look like.
import { expect, test } from '@playwright/test'

for (const [name, options] of [
  ['png', { type: 'png' }],
  ['jpeg-quality', { type: 'jpeg', quality: 80 }],
  ['full-page', { fullPage: true }],
  ['clip', { clip: { x: 10, y: 10, width: 80, height: 80 } }],
  ['transparent', { omitBackground: true }],
  ['css-scale', { scale: 'css' }],
] as const) {
  test(name, async ({ page, server }, info) => {
    const fixture = name === 'full-page' ? 'input/scrollable.html' : 'grid.html'
    await page.goto(`${server.PREFIX}/${fixture}`)
    const viewportHeight =
      name === 'full-page' ? await page.evaluate(() => innerHeight) : undefined
    const image = await page.screenshot(options)
    expect(image.length).toBeGreaterThan(0)
    if (viewportHeight !== undefined) {
      // PNG's IHDR stores the height as a big-endian uint32 at bytes 20–23.
      const height =
        image[20] * 0x1000000 +
        image[21] * 0x10000 +
        image[22] * 0x100 +
        image[23]
      expect(height).toBeGreaterThan(viewportHeight)
    }
    info.images.push({
      name,
      format: options.type || 'png',
      base64: image.toString('base64'),
    })
  })
}
test('locator', async ({ page, server }, info) => {
  await page.goto(`${server.PREFIX}/grid.html`)
  const image = await page.locator('body').screenshot()
  expect(image.length).toBeGreaterThan(0)
  info.images.push({
    name: 'locator',
    format: 'png',
    base64: image.toString('base64'),
  })
})
