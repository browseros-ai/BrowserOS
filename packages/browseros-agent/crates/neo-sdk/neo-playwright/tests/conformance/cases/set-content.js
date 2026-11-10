// @runner-p: stock-chromium
// page.setContent and frame.setContent: the document written and its load states, the HTML's
// script run in the main world with the write's user activation and the window's globals kept,
// the client's waitUntil check, waitUntil "networkidle", a timeout, a child frame's document
// replaced in process and out of it, and a detached frame's write.
const keep = (work) => harness.settle(work)
await page.goto(fixtures.url('child.html'))
await harness.pageEval(page, 'window.before = 1')
const written = await keep(
  page.setContent(
    '<!doctype html><title>Written</title><p id="w">written</p>' +
      '<script>window.activation = navigator.userActivation.isActive</script>',
  ),
)
const page_ = {
  written,
  title: await page.title(),
  text: await keep(page.locator('#w').textContent()),
  activation: await harness.pageEval(page, 'window.activation'),
  kept: await harness.pageEval(page, 'window.before'),
  url: page.url(),
  content: await page.content(),
  domcontentloaded: await keep(
    page.setContent('<title>D</title>', { waitUntil: 'domcontentloaded' }),
  ),
  commit: await keep(page.setContent('<title>C</title>', { waitUntil: 'commit' })),
  unknownWaitUntil: await keep(page.setContent('<p>x</p>', { waitUntil: 'nope' })),
  networkidle: await keep(page.setContent('<title>N</title>', { waitUntil: 'networkidle' })),
  timeout: await keep(
    page.setContent(`<img src="${fixtures.url('child.html?delay=3000')}">`, { timeout: 300 }),
  ),
}

await page.goto(fixtures.url('frames.html'))
await harness.poll(
  page,
  "document.querySelector('iframe[name=same]')?.contentDocument?.readyState === 'complete'",
)
await harness.poll(page, "document.querySelector('iframe[name=cross]') !== null")
const same = page.frame('same')
const cross = page.frame('cross')
await cross.waitForLoadState()
const frames = {
  same: await keep(same.setContent('<title>SameWritten</title>').then(() => same.title())),
  cross: await keep(cross.setContent('<title>CrossWritten</title>').then(() => cross.title())),
  crossScript: await keep(cross.evaluate(() => [document.title, location.hostname])),
  top: await page.title(),
}
await harness.pageEval(page, "document.querySelector('iframe[name=same]').remove(), true")
frames.detached = await keep(same.setContent('<p>x</p>', { timeout: 3000 }))
return { page: page_, frames }
