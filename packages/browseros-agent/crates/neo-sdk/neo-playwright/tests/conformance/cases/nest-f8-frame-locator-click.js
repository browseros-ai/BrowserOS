// F8 (design D22): clicks through frameLocator chains three and four levels deep
// (a1 > b2 > b3, and b1 > c2 > a3 > c4), then each frame's own log (frame.js mirrors
// window.__state into <html data-state>): the press and the click landed in the frame the
// chain names and nowhere else.
await page.goto(fixtures.url('nest/top.html'))
const b3 = page
  .frameLocator('iframe[title=a1]')
  .frameLocator('iframe[title=b2]')
  .frameLocator('iframe[title=b3]')
const c4 = page
  .frameLocator('iframe[title=b1]')
  .frameLocator('iframe[title=c2]')
  .frameLocator('iframe[title=a3]')
  .frameLocator('iframe[title=c4]')
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
const clicked = {
  b3: await click(b3.getByRole('button')),
  c4: await click(c4.getByRole('button', { name: 'C4 button' })),
}
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
