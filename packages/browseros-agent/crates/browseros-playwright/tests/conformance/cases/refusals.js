// What a script gets for a Playwright member Neo does not serve, and for an argument the client
// refuses before any call: a method (page.route), a getter (context.request) and a bad load
// state. Upstream serves the first two and throws the third with the same message. In Neo each
// refusal is a door call: the error keeps Playwright's message, the door's code reaches it once
// the script yields to Rust (here through page.title()), and its op id is the refusal's row in
// the run's trace, also for a refusal the script caught.
const record = (error) =>
  error === null
    ? null
    : {
        name: error.name,
        message: error.message.split('\n')[0],
        code: error.code ?? null,
        recorded: typeof error.operationId === 'string',
      }
const caught = async (work) => {
  try {
    await work()
    return null
  } catch (error) {
    return error
  }
}
await page.goto(fixtures.url('reads.html'))
const route = await caught(() => page.route('**/never', (handled) => handled.continue()))
const request = await caught(() => context.request)
const state = await caught(() => page.waitForLoadState('bogus'))
await page.title()
return { route: record(route), request: record(request), state: record(state) }
