// page.ariaSnapshot() in its default mode, on each page whose roles and names matter: the
// YAML Playwright prints, without refs.
const snapshots = {}
for (const name of ['reads', 'strict', 'select', 'editable', 'dialogs', 'popups']) {
  await page.goto(fixtures.url(`${name}.html`))
  snapshots[name] = await harness.settle(page.ariaSnapshot())
}
return snapshots
