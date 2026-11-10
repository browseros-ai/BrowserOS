// F3 (design D22), its click half: a click through each of the 8 button refs, then each
// frame's own log (frame.js mirrors window.__state into <html data-state>): exactly one press
// and one click on its own button, and nothing from another frame. nest-f3-fill.js is the
// fill half.
await page.goto(fixtures.url('nest/top.html'))
let snapshot = ''
for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 button'); attempt++)
  snapshot = await page.ariaSnapshot({ mode: 'ai' })
// Each button is hovered first, which scrolls it into view, and the layout gets 200 ms before
// the click. Right after a scroll in a cross-process ancestor the browser can route a press to
// that ancestor, and real Playwright then reports a click that never landed (seen in Runner
// P), which would make its outcome a coin flip; Neo checks the route before it presses, and
// actions_live.rs covers that race. Not scrollIntoViewIfNeeded: upstream's waits for a stable
// element on the frame's animation frames before it scrolls, and an offscreen cross-site
// frame may run none, so Runner P timed out there once.
const click = async (button) => {
  await button.hover()
  await page.waitForTimeout(200)
  return harness.settle(button.click())
}
const clicked = {}
for (const [, name, ref] of snapshot.matchAll(/- button "([^"]+)" \[ref=([^\]]+)\]/g))
  clicked[name] = await click(page.locator(`aria-ref=${ref}`))
const logs = {}
for (const frame of page.frames()) {
  const file = frame.url().split('?')[0].split('/').pop()
  logs[file] = await harness.settle(
    frame
      .locator('html')
      .getAttribute('data-state')
      .then((state) => {
        const { presses, clicks } = JSON.parse(state)
        return { presses, clicks }
      }),
  )
}
return { clicked, logs }
