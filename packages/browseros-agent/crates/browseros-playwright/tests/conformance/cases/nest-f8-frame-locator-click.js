// @needs R8 and evaluate: locator.click and frame.evaluate are not built
// F8 (design D22): clicks through frameLocator chains three and four levels deep
// (a1 > b2 > b3, and b1 > c2 > a3 > c4), then each frame's own log: the click landed in the
// frame the chain names and nowhere else.
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
const clicked = {
  b3: await harness.settle(b3.getByRole('button').click()),
  c4: await harness.settle(c4.getByRole('button', { name: 'C4 button' }).click()),
}
const logs = {}
for (const frame of page.frames()) {
  const file = new URL(frame.url()).pathname.split('/').pop()
  logs[file] = await harness.settle(
    frame.evaluate(() => window.__state.clicks),
  )
}
return { clicked, logs }
