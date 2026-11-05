/**
 * Conformance tier 1's case contract (design D15), evaluated as is by both runners so a case
 * means the same thing to each:
 *
 *   Runner P  runner-p.mjs                  real playwright-core 1.64.1 in Node, over CDP
 *   Runner N  tests/conformance_live.rs     Neo's prelude in QuickJS, over Playwright::invoke
 *
 * A case is one file, `cases/<name>.js`: the body of an async function, written as an agent
 * writes Playwright in Neo's `run` tool. It sees
 *
 *   page       a fresh tab, still blank; the case opens its page with `page.goto`
 *   context    the page's BrowserContext
 *   fixtures   fixtures.url(path, site) is a URL on the fixture server (server.mjs), where
 *              site 'a' is 127.0.0.1, 'b' is localhost and 'c' is [::1], three sites, so a
 *              frame on another site is an out-of-process iframe
 *   harness    test-only help no Playwright script has:
 *              pageEval(page, expression)  runs expression in the page's own main world with a
 *                                          user gesture, as the user or the page itself would,
 *                                          and resolves to its value as JSON
 *              poll(page, expression)      pageEval until the value is truthy (5 s)
 *              settle(promise)             { returned: value } or { threw: { name, message } },
 *                                          so one case can record several outcomes
 *
 * and returns JSON. A line `// @needs <unit>: <why>` makes both runners skip the case with
 * that reason, for a case written before its op is built; deleting the line enables it.
 *
 * The runner records one outcome per case: { returned }, { threw }, { skipped }, or
 * { broke } when the runner itself failed. diff.mjs compares the two runners' outcomes.
 */
globalThis.__conformance = (() => {
  const AsyncFunction = (async () => {}).constructor
  const SITES = { a: '127.0.0.1', b: 'localhost', c: '[::1]' }

  /** The reason a case is not run yet, from its `// @needs` line, or null. */
  function needs(source) {
    const line = /^\/\/ @needs (.+)$/m.exec(source)
    return line === null ? null : line[1].trim()
  }

  function errorOf(thrown) {
    return {
      name: String(thrown?.name ?? 'Error'),
      message: String(thrown?.message ?? thrown),
    }
  }

  async function settle(work) {
    try {
      return { returned: (await work) ?? null }
    } catch (thrown) {
      return { threw: errorOf(thrown) }
    }
  }

  const sleep = (ms) => new Promise((done) => setTimeout(done, ms))

  /**
   * Runs one case's source with the runner's `page`, `context`, fixture `port` and
   * `pageEval`, and resolves to its outcome, never rejecting for what the case threw.
   */
  async function run(source, { page, context, port, pageEval }) {
    const fixtures = {
      url: (path, site = 'a') =>
        `http://${SITES[site]}:${port}/${String(path).replace(/^\//, '')}`,
    }
    const poll = async (target, expression) => {
      for (let attempt = 0; attempt < 100; attempt++) {
        const value = await pageEval(target, expression)
        if (value) return value
        await sleep(50)
      }
      throw new Error(`harness.poll: ${expression} stayed falsy for 5 s`)
    }
    const harness = { pageEval, poll, settle }
    const body = new AsyncFunction(
      'page',
      'context',
      'fixtures',
      'harness',
      source,
    )
    return settle(body(page, context, fixtures, harness))
  }

  return { needs, run }
})()
