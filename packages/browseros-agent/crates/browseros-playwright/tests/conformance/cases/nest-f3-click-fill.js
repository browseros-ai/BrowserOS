// @needs R8, R9 and evaluate: locator.click, locator.fill and frame.evaluate are not built
// F3 (design D22): a click and a fill through each of the 16 refs, then each frame's own log
// (frame.js's window.__state): exactly one click on its own button and its own typed value,
// and nothing from another frame.
await page.goto(fixtures.url('nest/top.html'))
let snapshot = ''
for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 button'); attempt++)
  snapshot = await page.ariaSnapshot({ mode: 'ai' })
const acted = {}
for (const [, role, name, ref] of snapshot.matchAll(
  /- (button|textbox) "([^"]+)" \[ref=([^\]]+)\]/g,
)) {
  const target = page.locator(`aria-ref=${ref}`)
  acted[name] = await harness.settle(
    role === 'button' ? target.click() : target.fill(`${name} typed`),
  )
}
const logs = {}
for (const frame of page.frames()) {
  const file = new URL(frame.url()).pathname.split('/').pop()
  logs[file] = await harness.settle(frame.evaluate(() => window.__state))
}
return { acted, logs }
