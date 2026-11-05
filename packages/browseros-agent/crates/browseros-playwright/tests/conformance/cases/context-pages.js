// context.pages() and context.newPage(): the list grows by the new page and shrinks when it
// closes, the context hears the new page, and a closed page refuses work. Counts are
// relative, because the browser may hold tabs this run did not open.
await page.goto(fixtures.url('child.html'))
const before = context.pages()
const heard = []
context.on('page', (opened) => heard.push(opened))
const second = await context.newPage()
const facts = {
  includesPage: before.includes(page),
  newIsListed: context.pages().includes(second),
  grewBy: context.pages().length - before.length,
  newUrl: second.url(),
  heardIt: heard.length === 1 && heard[0] === second,
  sameContext: second.context() === context && page.context() === context,
  browserListsContext: context.browser()?.contexts().includes(context) ?? null,
}
await second.goto(fixtures.url('child.html?name=Second'))
facts.secondUrl = second.url()
const closeEvent = new Promise((done) => second.once('close', () => done(true)))
await second.close()
facts.closeEvent = await closeEvent
facts.isClosed = second.isClosed()
facts.shrankBackBy = context.pages().length - before.length
facts.stillListed = context.pages().includes(second)
facts.closeAgain = await harness.settle(second.close())
facts.gotoAfterClose = await harness.settle(
  second.goto(fixtures.url('child.html')),
)
facts.firstStillOpen = !page.isClosed() && context.pages().includes(page)
return facts
