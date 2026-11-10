// hover and dblclick: hover moves the pointer onto #hover (it sees the pointer arrive and
// move), dblclick then leaves it for the button, whose page sees both presses with their
// click counts and the dblclick. A hover and a dblclick on nothing time out.
await page.goto(fixtures.url('pointer.html'))
const hovered = await harness.settle(page.locator('#hover').hover())
const afterHover = await page.locator('#hover').textContent()
const doubled = await harness.settle(page.locator('#double').dblclick())
const afterLeave = await page.locator('#hover').textContent()
const presses = await harness.pageEval(
  page,
  `window.__events.filter((event) => event.target === '#double' && 'button' in event)`,
)
const missingHover = await harness.settle(page.locator('#nope').hover({ timeout: 300 }))
const missingDblclick = await harness.settle(
  page.locator('#nope').dblclick({ timeout: 300 }),
)
return { hovered, afterHover, doubled, afterLeave, presses, missingHover, missingDblclick }
