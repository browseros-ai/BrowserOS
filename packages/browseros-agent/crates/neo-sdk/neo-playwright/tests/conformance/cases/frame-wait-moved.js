// frame.waitForFunction on a child frame that navigates to another site mid-wait, and so moves
// to another renderer while it stays the same frame: out of process to a third site, out of
// process back to its parent's site, and in process out to another site. The wait goes on in
// the frame's new document, where alone its predicate is true. A frame the page removes
// mid-wait fails the wait.
const keep = (work) => harness.settle(work)
await page.goto(fixtures.url('frames.html'))
await harness.poll(
  page,
  "document.querySelector('iframe[name=same]')?.contentDocument?.readyState === 'complete'",
)
await harness.poll(page, "document.querySelector('iframe[name=cross]') !== null")
const frames = { same: page.frame('same'), cross: page.frame('cross') }
await frames.cross.waitForLoadState()
const moves = [
  ['cross', 'c', 'CrossToThird'],
  ['cross', 'a', 'CrossToParent'],
  ['same', 'b', 'SameToCross'],
]
const moved = []
for (const [name, site, label] of moves) {
  const url = fixtures.url(`child.html?name=${label}`, site)
  await harness.pageEval(
    page,
    `setTimeout(() => { document.querySelector('iframe[name=${name}]').src = ${JSON.stringify(url)} }, 300), true`,
  )
  moved.push(
    await keep(
      frames[name]
        .waitForFunction(
          (label) =>
            document.getElementById('name')?.textContent === label && [
              location.hostname,
              document.getElementById('name').textContent,
            ],
          label,
        )
        .then((handle) => handle.jsonValue()),
    ),
  )
}
await harness.pageEval(
  page,
  "setTimeout(() => document.querySelector('iframe[name=same]').remove(), 300), true",
)
const detached = await keep(frames.same.waitForFunction(() => false))
return { moved, detached }
