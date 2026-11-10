// The frame tree of frames.html (a same-site frame and a cross-site one) and reads inside
// each frame: through frameLocator, through a Frame from the tree, and through contentFrame.
await page.goto(fixtures.url('frames.html'))
await harness.poll(
  page,
  "document.querySelector('iframe[name=same]').contentDocument?.readyState === 'complete'",
)
const tree = (frame) => ({
  name: frame.name(),
  url: frame.url(),
  parent: frame.parentFrame()?.name() ?? null,
  children: frame.childFrames().map((child) => child.name()),
  detached: frame.isDetached(),
})
const cross = page.frame('cross')
return {
  frames: page.frames().map(tree),
  main: page.mainFrame() === page.frames()[0],
  byName: page.frame({ name: 'same' })?.url() ?? null,
  byUrl: page.frame({ url: /name=Cross/ })?.name() ?? null,
  frameLocator: await harness.settle(
    page.frameLocator('iframe[name=same]').locator('#name').textContent(),
  ),
  crossFrameLocator: await harness.settle(
    page.frameLocator('iframe[name=cross]').getByRole('button').textContent(),
  ),
  contentFrame: await harness.settle(
    page.locator('iframe[name=cross]').contentFrame().locator('#name').textContent(),
  ),
  frameLocate: await harness.settle(cross.locator('#name').textContent()),
  frameCount: await harness.settle(cross.locator('button').count()),
  frameText: await harness.settle(cross.textContent('#name')),
  missingFrame: await harness.settle(
    page.frameLocator('iframe[name=nothing]').locator('p').textContent({ timeout: 300 }),
  ),
}
