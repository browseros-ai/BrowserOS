// F9 (design D22): a click into an L2 frame that an overlay in its L1 parent covers. It waits,
// then fails naming the overlay, and no input reaches the button (events.js mirrors the L2
// frame's log into <html data-events>).
await page.goto(fixtures.url('covered-frame.html'))
const button = page
  .frameLocator('iframe[title=L1]')
  .frameLocator('iframe[title=L2]')
  .getByRole('button', { name: 'Child button' })
const clicked = await harness.settle(button.click({ timeout: 1500 }))
const l2 = page.frames().find((frame) => frame.url().includes('name=L2'))
return {
  clicked,
  l2Events: await harness.settle(
    l2
      .locator('html')
      .getAttribute('data-events')
      .then((events) => JSON.parse(events).map((event) => event.type)),
  ),
}
