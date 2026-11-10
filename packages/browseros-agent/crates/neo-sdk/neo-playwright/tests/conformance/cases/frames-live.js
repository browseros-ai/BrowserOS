// page.frames() stays live with no navigation between a change and the read: a frame the page
// attaches after load, frames that navigate (one an out-of-process frame swapped back
// in-process), frames the page removes, and a popup's frames, each read after
// page.waitForTimeout, on Frame objects the script kept.
const tree = (target) =>
  target.frames().map((frame) => ({
    name: frame.name(),
    url: frame.url(),
    parent: frame.parentFrame() === null ? null : frame.parentFrame().name() || 'main',
    detached: frame.isDetached(),
  }))
await page.goto(fixtures.url('frames.html'))
await harness.poll(
  page,
  "document.querySelector('iframe[name=same]').contentDocument?.readyState === 'complete'",
)
const same = page.frame('same')
const cross = page.frame('cross')
await harness.pageEval(
  page,
  `(() => {
    const frame = document.createElement('iframe')
    frame.name = 'late'
    frame.onload = () => { window.__late = true }
    frame.src = 'http://localhost:' + location.port + '/child.html?name=Late'
    document.body.append(frame)
    return true
  })()`,
)
await harness.poll(page, 'window.__late === true')
await page.waitForTimeout(300)
const attached = {
  tree: tree(page),
  children: page.mainFrame().childFrames().map((frame) => frame.name()),
  late: page.frame('late')?.url() ?? null,
}
await harness.pageEval(
  page,
  "document.querySelector('iframe[name=same]').src = '/child.html?name=Moved'; true",
)
await harness.poll(
  page,
  "document.querySelector('iframe[name=same]').contentDocument?.getElementById('name')?.textContent === 'Moved'",
)
await harness.pageEval(
  page,
  `(() => {
    const frame = document.querySelector('iframe[name=cross]')
    frame.onload = () => { window.__swapped = true }
    frame.src = '/child.html?name=SwappedIn'
    return true
  })()`,
)
await harness.poll(page, 'window.__swapped === true')
await page.waitForTimeout(300)
const navigated = {
  same: same.url(),
  cross: cross.url(),
  kept: page.frames().includes(same) && page.frames().includes(cross),
  readInSwapped: await harness.settle(cross.locator('#name').textContent()),
}
await harness.pageEval(
  page,
  "document.querySelector('iframe[name=late]').remove(); document.querySelector('iframe[name=same]').remove(); true",
)
await page.waitForTimeout(300)
const detached = {
  names: page.frames().map((frame) => frame.name()),
  same: same.isDetached(),
  sameByName: page.frame('same') === null,
}
const opened = page.waitForEvent('popup')
await harness.pageEval(page, "window.open('/frames.html'); true")
const popup = await opened
await harness.poll(
  popup,
  "document.readyState === 'complete' && document.querySelector('iframe[name=same]')?.contentDocument?.readyState === 'complete'",
)
await page.waitForTimeout(500)
return { attached, navigated, detached, popup: popup.frames().map((frame) => frame.name()) }
