// waitForLoadState after a goto that returned at commit: each state waits for its own point
// in slow.html's load, and a state already reached returns at once.
await page.goto(fixtures.url('slow.html'), { waitUntil: 'commit' })
const reached = {}
for (const state of ['domcontentloaded', 'load', 'networkidle']) {
  await page.waitForLoadState(state)
  reached[state] = await harness.pageEval(page, 'document.readyState')
}
await page.waitForLoadState()
reached.default = await harness.pageEval(page, 'document.readyState')
reached.badState = await harness.settle(page.waitForLoadState('ready'))
await page.goto(fixtures.url('slow.html?again'), { waitUntil: 'commit' })
reached.timeout = await harness.settle(
  page.waitForLoadState('load', { timeout: 200 }),
)
return reached
