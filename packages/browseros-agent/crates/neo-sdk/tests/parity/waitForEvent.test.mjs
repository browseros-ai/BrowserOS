// page.waitForEvent and context.waitForEvent through the pinned Playwright and through Neo, on
// fresh tests/pages/console.html tabs (events.mjs): a wait started before a click resolves with
// the message the click logs, on the page and on the context, and takes its listener away; a
// predicate (a function, `options.predicate`, or an async function) skips the events it
// rejects; and a wait rejects by kind on its own timeout, on the default timeout, when its
// predicate throws, and when its page closes, while a wait for 'close' gets the page itself; a
// null signal is no signal, and an object that is no AbortSignal throws TypeError before the
// wait listens (aborting signals: abort.test.mjs).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-event-console.spec.ts "CM-001 should work @smoke";
    // tests/library/browsercontext-events.spec.ts "BC-032 console event should work @smoke".
    name: 'a wait resolves with the message a click logs, on the page and on the context',
    path: '/console.html',
    source: `const describe = (message) => [message.type(), message.text(), message.page() === page]
    const [fromPage] = await Promise.all([page.waitForEvent('console'), page.click('#log')])
    const [fromContext] = await Promise.all([page.context().waitForEvent('console'), page.click('#log')])
    return {
      fromPage: describe(fromPage),
      fromContext: describe(fromContext),
      listenersLeft: [page.listenerCount('console'), page.context().listenerCount('console')],
    }`,
    expected: {
      fromPage: ['log', 'click 1', true],
      fromContext: ['log', 'click 2', true],
      listenersLeft: [0, 0],
    },
    probe: 'window.__logged',
    effect: 4,
  },
  {
    // page-event-console.spec.ts "CM-009 should have location for console API calls" (a
    // predicate); browsercontext-events.spec.ts "BC-035 console event should work in popup 2"
    // (a context predicate).
    name: 'a predicate skips the events it rejects',
    path: '/console.html',
    source: `const pick = (message) => message.text() === 'pick me'
    const [sync] = await Promise.all([page.waitForEvent('console', pick), page.click('#pick')])
    const [options] = await Promise.all([page.waitForEvent('console', { predicate: pick, timeout: 5000 }), page.click('#pick')])
    const asked = []
    const [asynchronous] = await Promise.all([
      page.waitForEvent('console', async (message) => {
        asked.push(message.text())
        await page.title()
        return pick(message)
      }),
      page.click('#pick'),
    ])
    const [fromContext] = await Promise.all([page.context().waitForEvent('console', { predicate: pick }), page.click('#pick')])
    return {
      texts: [sync, options, asynchronous, fromContext].map((message) => message.text()),
      asked,
      listenersLeft: [page.listenerCount('console'), page.context().listenerCount('console')],
    }`,
    expected: {
      texts: ['pick me', 'pick me', 'pick me', 'pick me'],
      asked: ['skip me', 'pick me'],
      listenersLeft: [0, 0],
    },
    probe: 'window.__logged',
    effect: 10,
  },
  {
    // tests/page/page-filechooser.spec.ts "FC-011 should respect timeout", "FC-012 should
    // respect default timeout when there is no custom timeout"; tests/library/page-close.spec.ts
    // "PA-044 should pass page to close event", "PA-045 should terminate network waiters";
    // tests/library/browsercontext-basic.spec.ts "BC-012 close() should abort waitForEvent".
    name: 'a wait rejects on timeouts, a throwing predicate, and its page closing',
    path: '/console.html',
    source: `const timeout = await settle(page.waitForEvent('console', { timeout: 300 }))
    const contextTimeout = await settle(page.context().waitForEvent('console', { timeout: 300 }))
    page.setDefaultTimeout(300)
    const defaultTimeout = await settle(page.waitForEvent('console'))
    const predicateDefaultTimeout = await settle(page.waitForEvent('console', () => false))
    page.setDefaultTimeout(30000)
    const [throwingPredicate] = await Promise.all([
      settle(page.waitForEvent('console', () => {
        throw new RangeError('bad predicate')
      })),
      page.click('#log'),
    ])
    const other = await page.context().newPage()
    const closing = other.waitForEvent('console').then(
      () => 'resolved',
      (error) => ({ error: errorCategory(error), saysClosed: /closed/.test(error.message) }),
    )
    const [closed] = await Promise.all([other.waitForEvent('close'), other.close()])
    return {
      timeout,
      contextTimeout,
      defaultTimeout,
      predicateDefaultTimeout,
      throwingPredicate,
      closing: await closing,
      closeEventIsPage: closed === other,
      listenersLeft: page.listenerCount('console'),
    }`,
    // Playwright's TargetClosedError keeps the name Error.
    expected: {
      timeout: { error: 'TimeoutError' },
      contextTimeout: { error: 'TimeoutError' },
      defaultTimeout: { error: 'TimeoutError' },
      predicateDefaultTimeout: { error: 'TimeoutError' },
      throwingPredicate: { error: 'RangeError' },
      closing: { error: 'Error', saysClosed: true },
      closeEventIsPage: true,
      listenersLeft: 0,
    },
    probe: 'window.__logged',
    effect: 3,
  },
  {
    // client/waiter.ts `Waiter.rejectOnTimeout` (`if (signal) {`): a null signal is no signal,
    // on the page and on the context.
    name: 'a null signal is no signal',
    path: '/console.html',
    source: `const [fromPage] = await Promise.all([page.waitForEvent('console', { signal: null }), page.click('#log')])
    const [fromContext] = await Promise.all([page.context().waitForEvent('console', { signal: null, timeout: 5000 }), page.click('#log')])
    return [fromPage.text(), fromContext.text()]`,
    expected: ['click 1', 'click 2'],
    probe: 'window.__logged',
    effect: 4,
  },
  {
    // client/waiter.ts `Waiter.rejectOnTimeout` (`if (signal) {`): an object that is no
    // AbortSignal (no `addEventListener`) throws Playwright's TypeError before the wait listens,
    // on the page and on the context. Aborted and aborting signals: abort.test.mjs.
    name: 'a non-signal object throws before the wait listens',
    path: '/console.html',
    source: `const page_ = await settle(page.waitForEvent('console', { signal: {} }))
    const context_ = await settle(page.context().waitForEvent('page', { signal: {} }))
    return { page_, context_, listeners: page.listenerCount('console') + page.context().listenerCount('page') }`,
    expected: {
      page_: { error: 'TypeError' },
      context_: { error: 'TypeError' },
      listeners: 0,
    },
  },
]

test('waitForEvent settles the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/console.html': fixture('console.html') },
    cases,
  })
})
