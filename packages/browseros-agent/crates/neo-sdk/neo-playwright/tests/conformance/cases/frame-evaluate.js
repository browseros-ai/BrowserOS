// frame.evaluate, frame.waitForFunction, frame.title and frame.content in a same-site child
// frame and an out-of-process one: where each script runs, a child's script reaching its parent
// and failing to reach across sites, arguments, a throw, and the frame forms of a frame the page
// removed.
const keep = (work) => harness.settle(work)
await page.goto(fixtures.url('frames.html'))
await harness.poll(
  page,
  "document.querySelector('iframe[name=same]')?.contentDocument?.readyState === 'complete'",
)
await harness.poll(page, "document.querySelector('iframe[name=cross]') !== null")
const same = page.frame('same')
const cross = page.frame('cross')
await cross.waitForLoadState()
const reached = {
  same: await keep(
    same.evaluate(() => [
      document.getElementById('name').textContent,
      parent.document.title,
      window === top,
    ]),
  ),
  cross: await keep(
    cross.evaluate(() => [document.getElementById('name').textContent, location.hostname]),
  ),
  crossToParent: await keep(
    cross.evaluate(() => {
      try {
        return parent.document.title
      } catch (error) {
        return error.name
      }
    }),
  ),
  arg: await keep(same.evaluate(([a, b]) => a * b, [6, 7])),
  thrown: await keep(same.evaluate(() => { throw new Error('in the frame') })),
  wait: await keep(
    same.waitForFunction(() => document.getElementById('name')?.textContent).then((handle) =>
      handle.jsonValue(),
    ),
  ),
  crossWait: await keep(cross.waitForFunction((x) => x, 7).then((handle) => handle.jsonValue())),
  titles: [await same.title(), await cross.title()],
  sameContent: await keep(same.content()),
  crossContent: await keep(cross.content()),
}
await harness.pageEval(page, "document.querySelector('iframe[name=same]').remove(), true")
const detached = {
  evaluate: await keep(same.evaluate(() => 1)),
  waitForFunction: await keep(same.waitForFunction(() => true)),
  content: await keep(same.content()),
  title: await keep(same.title()),
}
return { reached, detached }
