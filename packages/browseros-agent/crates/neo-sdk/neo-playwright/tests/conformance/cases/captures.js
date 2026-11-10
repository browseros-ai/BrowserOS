// page.screenshot and page.pdf on reads.html: the bytes' format and the image's size, never
// the pixels, which differ between runs.
await page.goto(fixtures.url('reads.html'))
const bytes = (data, count) => Array.from(data.slice(0, count))
const word = (data, at) =>
  ((data[at] << 24) | (data[at + 1] << 16) | (data[at + 2] << 8) | data[at + 3]) >>> 0
const png = await page.screenshot()
const full = await page.screenshot({ fullPage: true })
const jpeg = await page.screenshot({ type: 'jpeg', quality: 50 })
const pdf = await page.pdf()
return {
  pngSignature: bytes(png, 8),
  size: { width: word(png, 16), height: word(png, 20) },
  fullPage: { width: word(full, 16), height: word(full, 20) },
  jpegStart: bytes(jpeg, 3),
  pdfStart: String.fromCharCode(...bytes(pdf, 5)),
  // Scripts often send a screenshot on as base64, which Playwright's Buffer gives directly.
  base64Start: png.toString('base64').slice(0, 12),
  unknownType: await harness.settle(page.screenshot({ type: 'gif' })),
}
