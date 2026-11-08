// page.exposeBinding and browserContext.exposeBinding / exposeFunction through the Playwright
// reference and through Neo: the callback gets the call's `{ context, page, frame }` (a child
// frame's call names that frame) and its result answers the page; a context's binding reaches
// every page of the context, those it opens later included; a name registers once, with
// Playwright's three refusals. Each engine runs in a browser of its own (`isolate`), as both put
// Playwright's context-wide `__playwright__binding__` on every page of a shared browser.
// Values by `describe` (values.mjs), errors by category and the words Playwright's tests look for.
//
// Cases from tests/page/page-expose-function.spec.ts at 26137face: EF-001 "exposeBinding should
// work", EF-010 "should work on frames"; and tests/library/browsercontext-expose-function.spec.ts:
// EF-022 "expose binding should work", EF-023 "should work", EF-025 "should throw for duplicate
// registrations".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = '<title>binding</title><p id="p">text</p>'

test('exposeBinding gives the callback the call’s context, page and frame', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      let seen
      await page.exposeBinding('add', (source, a, b) => { seen = source; return a + b })
      const result = await page.evaluate(async () => window.add(5, 6))
      const top = { context: seen.context === page.context(), page: seen.page === page, frame: seen.frame === page.mainFrame() }
      await page.evaluate(() => new Promise((resolve) => {
        const frame = document.createElement('iframe')
        frame.src = location.pathname + '?child'
        frame.onload = resolve
        document.body.append(frame)
      }))
      const child = page.frames().find((frame) => frame.url().endsWith('?child'))
      const childResult = await child.evaluate(async () => window.add(1, 2))
      return { result, top, childResult, childFrame: seen.frame === child }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.deepEqual(outcome.value, {
    result: 11,
    top: { context: true, page: true, frame: true },
    childResult: 3,
    childFrame: true,
  })
})

test('a context’s binding reaches every page, and a name registers once', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const browserContext = page.context()
      await browserContext.exposeFunction('foo', () => 'foo')
      await browserContext.exposeFunction('bar', () => {})
      const dupContext = await outcome(() => browserContext.exposeFunction('foo', () => {}), ['Function "foo" has been already registered'])
      const second = await browserContext.newPage()
      const dupPage = await outcome(() => second.exposeFunction('foo', () => {}), ['has been already registered in the browser context'])
      await second.exposeFunction('baz', () => {})
      const dupPages = await outcome(() => browserContext.exposeFunction('baz', () => {}), ['has been already registered in one of the pages'])
      await second.goto(page.url())
      const fromSecond = await second.evaluate(async () => await window.foo())
      const fromFirst = await page.evaluate(async () => await window.foo())
      let seen
      await browserContext.exposeBinding('who', (source) => { seen = source; return source.page === page })
      const who = await page.evaluate(async () => await window.who())
      await second.close()
      return { dupContext, dupPage, dupPages, fromSecond, fromFirst, who, context: seen.context === browserContext }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.deepEqual(outcome.value.fromSecond, 'foo')
  assert.deepEqual(outcome.value.fromFirst, 'foo')
  assert.equal(outcome.value.who, true)
  assert.equal(outcome.value.context, true)
})
