// page.on('pageerror') and context.on('weberror') through the pinned Playwright and through Neo,
// on fresh tests/pages/page-error.html tabs (events.mjs): an Error a click handler throws
// reaches the listener, before the click resolves, as an Error with its name (an own `name`
// included) and message, and logs no console message; odd thrown values and an unhandled
// rejection become Errors named and worded as Playwright parses them; and the context's
// WebError, delivered before the page's event, holds the page, the same Error and where it was
// thrown. Stacks compare by type only.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-event-pageerror.spec.ts "ER-001 should fire", "ER-002 should not receive
    // console message for pageError", "ER-004 should contain the Error.name property".
    name: 'a thrown Error reaches the listener before the click resolves',
    path: '/page-error.html',
    source: `const errors = []
    page.on('pageerror', (error) => errors.push(error))
    const consoleTexts = []
    page.on('console', (message) => consoleTexts.push(message.text()))
    await page.click('#error')
    const atResolve = errors.length
    await page.click('#named')
    return {
      atResolve,
      errors: errors.map((error) => [error instanceof Error, error.name, error.message, typeof error.stack]),
      consoleTexts,
    }`,
    expected: {
      atResolve: 1,
      errors: [
        [true, 'Error', 'Fancy error!', 'string'],
        [true, 'my-name', 'my-message', 'string'],
      ],
      consoleTexts: [],
    },
    probe: 'window.__thrown',
    effect: 2,
  },
  {
    // page-event-pageerror.spec.ts "ER-006 should handle odd values", "ER-010 should emit error
    // from unhandled rejects".
    name: 'odd thrown values and a rejection become Errors as Playwright parses them',
    path: '/page-error.html',
    source: `const errors = {}
    for (const selector of ['#string', '#number', '#null', '#reject']) {
      const [error] = await Promise.all([page.waitForEvent('pageerror'), page.click(selector)])
      errors[selector] = [error instanceof Error, error.name, error.message]
    }
    return errors`,
    // A value that is not an Error has no name; its message is the value as a string.
    expected: {
      '#string': [true, '', 'oops'],
      '#number': [true, '', '42'],
      '#null': [true, '', 'null'],
      '#reject': [true, 'Error', 'sad :('],
    },
    probe: 'window.__thrown',
    effect: 4,
  },
  {
    // tests/library/browsercontext-events.spec.ts "BC-043 weberror event should work",
    // "BC-044 weberror event should include location".
    name: "the context's weberror holds the page, the same Error and where it was thrown",
    path: '/page-error.html',
    source: `const order = []
    let fromPage
    page.on('pageerror', (error) => {
      order.push('pageerror')
      fromPage = error
    })
    page.context().on('weberror', (webError) => {
      if (webError.page() === page) order.push('weberror')
    })
    const [webError] = await Promise.all([page.context().waitForEvent('weberror'), page.click('#error')])
    const error = webError.error()
    const location = webError.location()
    return {
      page: webError.page() === page,
      error: [error instanceof Error, error.name, error.message],
      sameError: error === fromPage,
      location: { ...location, url: path(location.url) },
      order,
    }`,
    // The location is zero-based, as CDP reports the throw in page-error.html, and has no
    // lineNumber or columnNumber, unlike a ConsoleMessage's.
    expected: {
      page: true,
      error: [true, 'Error', 'Fancy error!'],
      sameError: true,
      location: { url: '/page-error.html', line: 7, column: 105 },
      order: ['weberror', 'pageerror'],
    },
    probe: 'window.__thrown',
    effect: 1,
  },
]

test('page errors reach the same listeners as the same Errors through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/page-error.html': fixture('page-error.html') },
    cases,
  })
})
