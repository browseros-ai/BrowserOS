// console and pageerror on a hosted BrowserOS (run.sh starts it hosted, as Neo runs), which
// sends neither to any client: what waitForEvent gets for a console.log and a thrown error.
await page.goto(fixtures.url('child.html'))
const message = page.waitForEvent('console', { timeout: 1000 })
await harness.pageEval(page, "console.log('hello', 7)")
const consoleEvent = await harness.settle(
  message.then((logged) => ({ type: logged.type(), text: logged.text() })),
)
const thrown = page.waitForEvent('pageerror', { timeout: 1000 })
await harness.pageEval(page, "setTimeout(() => { throw new Error('boom') })")
const pageError = await harness.settle(thrown.then((error) => error.message))
return { consoleEvent, pageError }
