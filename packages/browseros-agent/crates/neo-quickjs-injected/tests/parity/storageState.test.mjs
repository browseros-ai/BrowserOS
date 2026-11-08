// context.storageState through the pinned Playwright and through Neo, on fresh tabs of an inline
// page (events.mjs): the state is { cookies, origins }, its cookies as cookies() reads them, and
// the localStorage of each origin the page visited: the one it is on, and the one it left
// (server/browserContext.ts `storageState` reads a left origin from a page of its own, which no
// 'page' listener hears).
//
// Playwright lists an origin once its client saw a page commit a document there
// (`addVisitedOrigin`; a document open before it attached is not one), so each script navigates
// before it reads. Neo's context is the user's own profile, whose state holds every cookie and
// origin the user has: a script keeps only the cookies and items it named (`neo-ss-`) on the
// fixture's two origins (127.0.0.1, and localhost, another site), and removes them before it
// returns. Origins compare by which of the two they are, never by port. Neo refuses `path`: it
// writes a script's files only inside the run's artifact directory, and this run has none; that
// case runs through Neo alone.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/**
 * `ours(cookies)`: the case's cookies by name; `named(items)`: the case's storage items;
 * `entryOf(state, at, which)`: the state's entry for an origin, named `which`; `forget(...)`:
 * removes cookies by name.
 */
const STATE = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const otherOrigin = origin.replace('127.0.0.1', 'localhost')
    const start = page.url()
    const named = (items) => items.filter((item) => item.name.startsWith('neo-ss-'))
    const ours = (cookies) => named(cookies).sort((a, b) => (a.name < b.name ? -1 : 1))
    const entryOf = (state, at, which) => {
      const entry = state.origins.find((candidate) => candidate.origin === at)
      return entry ? { ...entry, origin: which, localStorage: named(entry.localStorage) } : 'not listed'
    }
    const forget = async (...names) => {
      for (const name of names) await page.context().clearCookies({ name })
    }
    `

const PROBE = `[location.pathname, localStorage.getItem('neo-ss-name1'),
  document.cookie.split('; ').filter((pair) => pair.startsWith('neo-ss-')).join('; ')]`

const cases = [
  {
    // tests/library/browsercontext-storage-state.spec.ts "ST-006 should capture cookies", "ST-001
    // should capture local storage" (the origin the page is on).
    name: "storageState holds the context's cookies and the localStorage of the page's origin",
    path: '/storageState.html',
    source: `${STATE}try {
      await page.goto(origin + '/set-cookies')
      await page.evaluate(() => localStorage.setItem('neo-ss-name1', 'value1'))
      const state = await page.context().storageState()
      return {
        keys: Object.keys(state).sort(),
        cookies: ours(state.cookies),
        current: entryOf(state, origin, 'current'),
      }
    } finally {
      await page.evaluate(() => localStorage.removeItem('neo-ss-name1'))
      await forget('neo-ss-a', 'neo-ss-empty')
      await page.goto(start)
    }`,
    expected: {
      keys: ['cookies', 'origins'],
      cookies: [
        {
          name: 'neo-ss-a',
          value: 'b',
          domain: '127.0.0.1',
          path: '/',
          expires: -1,
          httpOnly: false,
          secure: false,
          sameSite: 'Lax',
        },
        {
          name: 'neo-ss-empty',
          value: '',
          domain: '127.0.0.1',
          path: '/',
          expires: -1,
          httpOnly: false,
          secure: false,
          sameSite: 'Lax',
        },
      ],
      current: {
        origin: 'current',
        localStorage: [{ name: 'neo-ss-name1', value: 'value1' }],
      },
    },
    probe: PROBE,
    effect: ['/storageState.html', null, ''],
  },
  {
    // browsercontext-storage-state.spec.ts "ST-001 should capture local storage" (an origin the
    // page left too), "ST-007 should not emit events about internal page".
    name: 'storageState also holds the localStorage of an origin the page left, and opens no page',
    path: '/storageState.html',
    source: `${STATE}try {
      await page.goto(start)
      await page.evaluate(() => localStorage.setItem('neo-ss-name1', 'value1'))
      await page.goto(otherOrigin + '/storageState.html')
      await page.evaluate(() => localStorage.setItem('neo-ss-name2', 'value2'))
      const opened = []
      const onPage = (other) => opened.push(path(other.url()))
      page.context().on('page', onPage)
      const state = await page.context().storageState()
      page.context().off('page', onPage)
      return { left: entryOf(state, origin, 'left'), current: entryOf(state, otherOrigin, 'current'), opened }
    } finally {
      await page.goto(otherOrigin + '/storageState.html')
      await page.evaluate(() => localStorage.removeItem('neo-ss-name2'))
      await page.goto(start)
      await page.evaluate(() => localStorage.removeItem('neo-ss-name1'))
    }`,
    expected: {
      left: {
        origin: 'left',
        localStorage: [{ name: 'neo-ss-name1', value: 'value1' }],
      },
      current: {
        origin: 'current',
        localStorage: [{ name: 'neo-ss-name2', value: 'value2' }],
      },
      opened: [],
    },
    // Deviation (neo-runtime session_state.rs `storage_state`): Playwright reads the origin the
    // page left in a stubbed tab of its own; Neo reads only origins the run has a page open on,
    // and opens no tab in the user's browser.
    deviation: (outcome) =>
      assert.deepEqual(outcome.value, {
        left: 'not listed',
        current: {
          origin: 'current',
          localStorage: [{ name: 'neo-ss-name2', value: 'value2' }],
        },
        opened: [],
      }),
    probe: PROBE,
    effect: ['/storageState.html', null, ''],
  },
  {
    // browsercontext-storage-state.spec.ts "ST-004 should round-trip through the file", which
    // Neo refuses: Playwright writes the state at `path` on its host; Neo writes a script's
    // files only inside the run's artifact directory, and names it when the run has none. The
    // state itself is still served. Neo only.
    name: 'Neo refuses storageState({ path }) in a run with no artifact directory',
    path: '/storageState.html',
    source: `const refused = await page.context().storageState({ path: 'state.json' }).then(
      () => 'resolved',
      (error) => ({ error: errorCategory(error), namesArtifactDirectory: /artifact director/i.test(error.message) }),
    )
    const state = await page.context().storageState()
    return { refused, served: Array.isArray(state.cookies) && Array.isArray(state.origins) }`,
    neo: (outcome) => {
      assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
      assert.deepEqual(outcome.value, {
        refused: { error: 'Error', namesArtifactDirectory: true },
        served: true,
      })
    },
  },
]

test('context.storageState reads the same state through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/storageState.html': '<title>storageState</title><h1>storageState</h1>',
      '/set-cookies': {
        headers: {
          'set-cookie': ['neo-ss-a=b; Path=/', 'neo-ss-empty=; Path=/'],
        },
        body: '<title>set-cookies</title><h1>set-cookies</h1>',
      },
    },
    cases,
  })
})
