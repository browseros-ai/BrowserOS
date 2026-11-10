// goto at each waitUntil on slow.html, where commit, DOMContentLoaded and load are held
// hundreds of ms apart: what the page has reached when goto returns. At commit the page's
// first script may not have run yet, so only its readyState is read there.
const states = {}
for (const waitUntil of ['commit', 'domcontentloaded', 'load', 'networkidle']) {
  const response = await page.goto(fixtures.url(`slow.html?${waitUntil}`), {
    waitUntil,
  })
  states[waitUntil] = {
    status: response?.status() ?? null,
    readyState: await harness.pageEval(page, 'document.readyState'),
  }
  if (waitUntil !== 'commit')
    states[waitUntil].events = await harness.pageEval(
      page,
      'window.__events.map((event) => event.type)',
    )
}
// The default is load.
await page.goto(fixtures.url('slow.html?default'))
states.default = await harness.pageEval(page, 'document.readyState')
return states
