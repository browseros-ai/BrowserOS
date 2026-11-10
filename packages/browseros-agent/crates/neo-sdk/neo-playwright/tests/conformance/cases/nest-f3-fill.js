// F3 (design D22), its fill half: a fill through each of the 8 textbox refs, then each
// frame's own typed values (frame.js mirrors window.__state into <html data-state>): its own
// field's value, and nothing from another frame. nest-f3-click.js is the click half.
await page.goto(fixtures.url('nest/top.html'))
let snapshot = ''
for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 field'); attempt++)
  snapshot = await page.ariaSnapshot({ mode: 'ai' })
const filled = {}
for (const [, name, ref] of snapshot.matchAll(/- textbox "([^"]+)" \[ref=([^\]]+)\]/g))
  filled[name] = await harness.settle(page.locator(`aria-ref=${ref}`).fill(`${name} typed`))
const values = {}
for (const frame of page.frames()) {
  const file = frame.url().split('?')[0].split('/').pop()
  values[file] = await harness.settle(
    frame
      .locator('html')
      .getAttribute('data-state')
      .then((state) => JSON.parse(state).values),
  )
}
return { filled, values }
