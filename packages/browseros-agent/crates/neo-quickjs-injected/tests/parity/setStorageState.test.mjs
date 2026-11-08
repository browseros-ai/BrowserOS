// context.setStorageState through the pinned Playwright and through Neo, on fresh tabs of an
// inline page (events.mjs): the state's cookies reach the page, and a listed origin's
// localStorage becomes the state's, its earlier items gone, with no page opened or left open
// (server/browserContext.ts `setStorageState` restores an origin from a page of its own); what
// storageState returns, setStorageState puts back; a state file that does not exist rejects
// with Error before anything changes.
//
// Playwright also clears every other cookie and every other visited origin's storage first.
// Neo's context is the user's own profile, so no script reads anything it did not set: each
// keeps to the cookies and items it named (`neo-sss-`) on the page's own origin, and removes them
// before it returns.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/**
 * `read()`: the case's document.cookie pairs and storage items, as the page sees them;
 * `cleanUp()`: removes them.
 */
const STATE = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const mine = (name) => name.startsWith('neo-sss-')
    const read = () => page.evaluate(() => ({
      cookie: document.cookie.split('; ').filter((pair) => pair.startsWith('neo-sss-')).sort(),
      storage: Object.keys(localStorage).filter((name) => name.startsWith('neo-sss-')).sort()
        .map((name) => name + '=' + localStorage.getItem(name)),
    }))
    const cleanUp = async (...cookies) => {
      for (const name of cookies) await page.context().clearCookies({ name })
      await page.evaluate(() => Object.keys(localStorage).filter((name) => name.startsWith('neo-sss-'))
        .forEach((name) => localStorage.removeItem(name)))
    }
    `

const PROBE = `[document.cookie.split('; ').filter((pair) => pair.startsWith('neo-sss-')).join('; '),
  Object.keys(localStorage).filter((name) => name.startsWith('neo-sss-')).join()]`

const cases = [
  {
    // tests/library/browsercontext-storage-state.spec.ts "ST-002 should set local storage" (the
    // state replaces the origin's items, and context.pages() keeps its length).
    name: "setStorageState sets the state's cookies and replaces a listed origin's items, opening no page",
    path: '/setStorageState.html',
    source: `${STATE}try {
      await page.evaluate(() => localStorage.setItem('neo-sss-before', 'old'))
      const pages = page.context().pages().length
      const opened = []
      const onPage = (other) => opened.push(path(other.url()))
      page.context().on('page', onPage)
      const set = await settle(page.context().setStorageState({
        cookies: [{
          name: 'neo-sss-cookie', value: 'set', domain: '127.0.0.1', path: '/',
          expires: -1, httpOnly: false, secure: false, sameSite: 'Lax',
        }],
        origins: [{ origin, localStorage: [{ name: 'neo-sss-after', value: 'new' }] }],
      }))
      page.context().off('page', onPage)
      const samePages = page.context().pages().length === pages
      await page.reload()
      const stored = (await page.context().cookies(origin + '/')).filter((cookie) => mine(cookie.name))
      return { set, samePages, opened, page: await read(), stored }
    } finally {
      await cleanUp('neo-sss-cookie')
    }`,
    expected: {
      set: 'undefined',
      samePages: true,
      opened: [],
      page: { cookie: ['neo-sss-cookie=set'], storage: ['neo-sss-after=new'] },
      stored: [
        {
          name: 'neo-sss-cookie',
          value: 'set',
          domain: '127.0.0.1',
          path: '/',
          expires: -1,
          httpOnly: false,
          secure: false,
          sameSite: 'Lax',
        },
      ],
    },
    probe: PROBE,
    effect: ['', ''],
  },
  {
    // browsercontext-storage-state.spec.ts "ST-004 should round-trip through the file" (here
    // without the file: the state object itself), "ST-006 should capture cookies".
    name: 'what storageState returns, setStorageState puts back',
    path: '/setStorageState.html',
    source: `${STATE}const start = page.url()
    try {
      // A page that sets a cookie and an item of its own.
      await page.goto(origin + '/trip.html')
      const state = await page.context().storageState()
      const saved = {
        cookies: state.cookies.filter((cookie) => mine(cookie.name)),
        origins: state.origins.filter((entry) => entry.origin === origin)
          .map((entry) => ({ ...entry, localStorage: entry.localStorage.filter((item) => mine(item.name)) })),
      }
      await cleanUp('neo-sss-trip')
      const cleared = await read()
      await page.context().setStorageState(saved)
      await page.goto(start)
      return { cleared, restored: await read() }
    } finally {
      await cleanUp('neo-sss-trip')
      await page.goto(start)
    }`,
    expected: {
      cleared: { cookie: [], storage: [] },
      restored: {
        cookie: ['neo-sss-trip=cookie'],
        storage: ['neo-sss-trip=storage'],
      },
    },
    probe: PROBE,
    effect: ['', ''],
  },
  {
    // browsercontext-storage-state.spec.ts "ST-022 setStorageState should handle missing file".
    name: 'setStorageState with a state file that does not exist rejects with Error and changes nothing',
    path: '/setStorageState.html',
    source: `${STATE}try {
      await page.context().addCookies([{ url: origin + '/', name: 'neo-sss-untouched', value: '1' }])
      await page.evaluate(() => localStorage.setItem('neo-sss-untouched', '2'))
      const refused = await settle(page.context().setStorageState('neo-parity-missing/state.json'))
      return { refused, kept: await read() }
    } finally {
      await cleanUp('neo-sss-untouched')
    }`,
    expected: {
      refused: { error: 'Error' },
      kept: {
        cookie: ['neo-sss-untouched=1'],
        storage: ['neo-sss-untouched=2'],
      },
    },
    probe: PROBE,
    effect: ['', ''],
  },
]

test('context.setStorageState sets the same state through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/setStorageState.html':
        '<title>setStorageState</title><h1>setStorageState</h1>',
      '/trip.html': {
        headers: { 'set-cookie': 'neo-sss-trip=cookie; Path=/' },
        body: `<title>trip</title><script>localStorage.setItem('neo-sss-trip', 'storage')</script>`,
      },
    },
    cases,
  })
})
