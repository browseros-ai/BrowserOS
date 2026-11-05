// check, uncheck and setChecked (upstream's _setChecked): a checkbox on, again (no click), off;
// one checked through its label; radios, where uncheck clicks and the state does not change;
// a checkbox whose click handler cancels the change; and a button, which has no checked state.
// Then every input's state and the clicks the page received.
await page.goto(fixtures.url('checks.html'))
const steps = {
  check: await harness.settle(page.locator('#box').check()),
  checkAgain: await harness.settle(page.locator('#box').check()),
  uncheck: await harness.settle(page.locator('#box').uncheck()),
  label: await harness.settle(page.getByText('Labelled').check()),
  setChecked: await harness.settle(page.locator('#first').setChecked(true)),
  otherRadio: await harness.settle(page.getByLabel('Second').check()),
  uncheckRadio: await harness.settle(page.locator('#second').uncheck({ timeout: 1000 })),
  cancelled: await harness.settle(page.locator('#stuck').check({ timeout: 1000 })),
  notCheckable: await harness.settle(page.locator('#plain').check({ timeout: 1000 })),
  labelledIsChecked: await harness.settle(page.locator('#labelled').isChecked()),
}
const checked = await harness.pageEval(
  page,
  `Object.fromEntries([...document.querySelectorAll('input')].map((input) => [input.id, input.checked]))`,
)
const clicks = await harness.pageEval(
  page,
  `window.__events.filter((event) => event.type === 'click').map((event) => event.target)`,
)
return { steps, checked, clicks }
