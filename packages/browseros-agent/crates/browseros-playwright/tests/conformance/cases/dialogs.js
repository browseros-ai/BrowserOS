// Dialogs on dialogs.html, each opened by a click with a user gesture: what the dialog says,
// the answer the page got, through waitForEvent and through page.once('dialog'), and a
// confirm nobody listens for.
await page.goto(fixtures.url('dialogs.html'))
let asked = 0
const answered = async () => {
  asked += 1
  return JSON.parse(
    await harness.poll(
      page,
      `window.__answers.length >= ${asked} && JSON.stringify(window.__answers[${asked - 1}])`,
    ),
  )
}
const click = (id) =>
  harness.pageEval(page, `document.getElementById('${id}').click()`)
const viaWait = async (id, respond) => {
  const dialog = page.waitForEvent('dialog', { timeout: 3000 })
  await click(id)
  return harness.settle(
    dialog.then(async (opened) => {
      const seen = {
        type: opened.type(),
        message: opened.message(),
        defaultValue: opened.defaultValue(),
      }
      await respond(opened)
      seen.answer = await answered()
      return seen
    }),
  )
}
const results = {
  alert: await viaWait('alert', (dialog) => dialog.accept()),
  confirmAccepted: await viaWait('confirm', (dialog) => dialog.accept()),
  confirmDismissed: await viaWait('confirm', (dialog) => dialog.dismiss()),
  promptTyped: await viaWait('prompt', (dialog) => dialog.accept('Grace')),
  promptDefault: await viaWait('prompt', (dialog) => dialog.accept()),
  promptDismissed: await viaWait('prompt', (dialog) => dialog.dismiss()),
}
const once = new Promise((done) =>
  page.once('dialog', async (dialog) => {
    await dialog.accept()
    done(dialog.type())
  }),
)
await click('confirm')
results.onceListener = { type: await once, answer: await answered() }
await click('confirm')
results.unheardConfirm = await answered()
return results
