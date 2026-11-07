// page.consoleMessages, page.pageErrors and their clear methods through the pinned Playwright
// and through Neo, on fresh tests/pages/console.html tabs (events.mjs), which log twice and
// throw once while they load, before either script starts; Neo attaches to its tab only once
// the tab has loaded. Both keep what the load logged and threw, each message with its type,
// text, page and location, and never hand it to a listener the script adds; clearing empties
// each store and keeps what comes after; and the default filter, since-navigation, keeps only
// the current document's entries where 'all' keeps every document's, while a filter that is
// neither rejects.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-event-console.spec.ts "CM-016 should have timestamp in consoleMessages",
    // "CM-017 consoleMessages should work"; tests/page/page-event-pageerror.spec.ts "ER-011
    // pageErrors should work".
    name: 'what the load logged and threw is stored, and never reaches a later listener',
    path: '/console.html',
    source: `const heard = []
    page.on('console', (message) => heard.push(message.text()))
    page.on('pageerror', (error) => heard.push(error.message))
    const stored = await page.consoleMessages()
    const errors = await page.pageErrors()
    await page.click('#log')
    return {
      stored: stored.map((message) => [message.type(), message.text(), message.page() === page, message.worker(), typeof message.timestamp()]),
      locations: stored.map((message) => {
        const { url, ...at } = message.location()
        return { url: path(url), ...at }
      }),
      errors: errors.map((error) => [error instanceof Error, error.name, error.message]),
      heard,
      afterClick: (await page.consoleMessages()).map((message) => message.text()),
    }`,
    expected: {
      stored: [
        ['log', 'loaded first', true, null, 'number'],
        ['warning', 'load warning', true, null, 'number'],
      ],
      locations: [
        {
          url: '/console.html',
          line: 8,
          column: 8,
          lineNumber: 8,
          columnNumber: 8,
        },
        {
          url: '/console.html',
          line: 9,
          column: 8,
          lineNumber: 9,
          columnNumber: 8,
        },
      ],
      errors: [[true, 'Error', 'load error first']],
      heard: ['click 1'],
      afterClick: ['loaded first', 'load warning', 'click 1'],
    },
    probe: 'window.__logged',
    effect: 3,
  },
  {
    // page-event-console.spec.ts "CM-018 clearConsoleMessages should work";
    // page-event-pageerror.spec.ts "ER-012 clearPageErrors should work".
    name: 'clearing empties each store and keeps what comes after',
    path: '/console.html',
    source: `const stores = async () => ({
      console: (await page.consoleMessages()).map((message) => message.text()),
      errors: (await page.pageErrors()).map((error) => error.message),
    })
    const cleared = [await settle(page.clearConsoleMessages()), await settle(page.clearPageErrors())]
    const empty = await stores()
    await Promise.all([page.waitForEvent('console'), page.click('#log')])
    await Promise.all([page.waitForEvent('pageerror'), page.click('#throw')])
    return { cleared, empty, later: await stores() }`,
    expected: {
      cleared: ['undefined', 'undefined'],
      empty: { console: [], errors: [] },
      later: { console: ['click 1'], errors: ['click error'] },
    },
    probe: 'window.__logged',
    effect: 3,
  },
  {
    // page-event-console.spec.ts "CM-019 consoleMessages since-navigation filter should
    // work", "CM-020 pageErrors since-navigation filter should work".
    name: "since-navigation keeps the current document's entries, 'all' keeps every one",
    path: '/console.html',
    source: `const texts = async (options) => (await page.consoleMessages(options)).map((message) => message.text())
    const errors = async (options) => (await page.pageErrors(options)).map((error) => error.message)
    await page.click('#log')
    await page.goto(page.url() + '?second')
    return {
      all: await texts({ filter: 'all' }),
      sinceNavigation: await texts(),
      sinceNavigationNamed: await texts({ filter: 'since-navigation' }),
      errorsAll: await errors({ filter: 'all' }),
      errorsSinceNavigation: await errors(),
      badFilter: await settle(page.consoleMessages({ filter: 'today' })),
      badErrorsFilter: await settle(page.pageErrors({ filter: 'today' })),
    }`,
    expected: {
      all: [
        'loaded first',
        'load warning',
        'click 1',
        'loaded second',
        'load warning',
      ],
      sinceNavigation: ['loaded second', 'load warning'],
      sinceNavigationNamed: ['loaded second', 'load warning'],
      errorsAll: ['load error first', 'load error second'],
      errorsSinceNavigation: ['load error second'],
      badFilter: { error: 'Error' },
      badErrorsFilter: { error: 'Error' },
    },
    probe: 'window.__logged',
    effect: 2,
  },
]

test('stored console messages and page errors read the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/console.html': fixture('console.html') },
    cases,
  })
})
