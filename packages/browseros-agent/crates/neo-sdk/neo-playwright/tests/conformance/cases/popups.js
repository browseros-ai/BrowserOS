// Every way popups.html opens a page, each clicked with a user gesture and caught with
// page.waitForEvent('popup'): the popup's URL once loaded, its opener, whether the context
// lists it, and its close. page.on('popup') and context.on('page') see the same pages.
await page.goto(fixtures.url('popups.html'))
const heard = { popup: [], page: [] }
page.on('popup', (opened) => heard.popup.push(opened))
context.on('page', (opened) => heard.page.push(opened))
const caught = []
const open = async (id) => {
  const popup = page.waitForEvent('popup', { timeout: 3000 })
  await harness.pageEval(page, `document.getElementById('${id}').click()`)
  return harness.settle(
    popup.then(async (opened) => {
      caught.push(opened)
      await opened.waitForLoadState()
      const opener = await opened.opener()
      const seen = {
        url: opened.url(),
        opener: opener === page ? 'page' : opener === null ? null : 'another page',
        listed: context.pages().includes(opened),
      }
      await opened.close()
      seen.closed = opened.isClosed()
      seen.listedAfterClose = context.pages().includes(opened)
      return seen
    }),
  )
}
const kinds = {}
for (const id of ['tab', 'noopener', 'blank', 'blank-noopener'])
  kinds[id] = await open(id)
const tabs = {
  heardPopups: heard.popup.length,
  heardPages: heard.page.length,
  sameObjects: caught.every(
    (opened) => heard.popup.includes(opened) && heard.page.includes(opened),
  ),
}
// Last, as it opens a separate window rather than a tab.
kinds.window = await open('window')
return { kinds, tabs }
