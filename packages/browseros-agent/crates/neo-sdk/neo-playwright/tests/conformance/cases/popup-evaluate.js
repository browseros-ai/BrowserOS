// A popup and its opener read each other's password through window.opener and the window
// window.open returned, each tab's page script having set its own, and both come back raw, as
// upstream returns them, with what each script reads of its own page and of the other tab's text.
await page.goto(fixtures.url('sensitive.html'))
const opened = page.waitForEvent('popup', { timeout: 5000 })
await harness.pageEval(page, "(window.popup = window.open('child.html?name=Popup'), true)")
const popup = await opened
await popup.waitForLoadState()
await harness.pageEval(
  popup,
  "(document.body.insertAdjacentHTML('beforeend', '<input id=\"secret\" type=\"password\">'), " +
    "document.getElementById('secret').value = 'popup-secret-77', true)",
)
const keep = (work) => harness.settle(work)
return {
  popupReadsOpener: await keep(
    popup.evaluate(() => [
      document.getElementById('name').textContent,
      opener.document.title,
      opener.document.getElementById('password').value,
    ]),
  ),
  openerReadsPopup: await keep(
    page.evaluate(() => [
      document.title,
      window.popup.document.getElementById('name').textContent,
      window.popup.document.getElementById('secret').value,
    ]),
  ),
  popupOpener: (await popup.opener()) === page,
}
