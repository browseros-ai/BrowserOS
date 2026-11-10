// Clicks that wait (design D6): actionability.html makes each button actionable 300 ms after
// load, and each click waits until its button is shown, still, enabled, uncovered, and clear
// of a sticky header once scrolled. Then a right click, a click with Shift held, a trial click
// (which must not click), what the page received, and a click on nothing with its timeout.
await page.goto(fixtures.url('actionability.html?after=300'))
const clicked = {}
for (const id of ['appear', 'moving', 'enable', 'covered', 'under-sticky'])
  clicked[id] = await harness.settle(page.locator(`#${id}`).click())
const right = await harness.settle(page.locator('#enable').click({ button: 'right' }))
const shifted = await harness.settle(
  page.locator('#appear').click({ modifiers: ['Shift'] }),
)
const trial = await harness.settle(page.locator('#covered').click({ trial: true }))
const events = await harness.pageEval(
  page,
  `window.__events.filter((event) => !['DOMContentLoaded', 'load'].includes(event.type))`,
)
const missing = await harness.settle(page.locator('#nope').click({ timeout: 300 }))
return { clicked, right, shifted, trial, events, missing }
