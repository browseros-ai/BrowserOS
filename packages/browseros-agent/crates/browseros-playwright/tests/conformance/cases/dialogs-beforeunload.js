// A beforeunload dialog from page.close({ runBeforeUnload: true }) on a page that asks before
// it is left: the dialog's type, and the close once it is accepted. The page needs a user
// gesture first, or the browser skips the dialog.
await page.goto(fixtures.url('dialogs.html?beforeunload'))
await harness.pageEval(page, "document.getElementById('alert').focus()")
const dialog = page.waitForEvent('dialog', { timeout: 3000 })
const closed = new Promise((done) => page.once('close', () => done(true)))
await page.close({ runBeforeUnload: true })
return {
  dialog: await harness.settle(
    dialog.then(async (opened) => {
      const type = opened.type()
      await opened.accept()
      return type
    }),
  ),
  closeEvent: await closed,
  isClosed: page.isClosed(),
}
