// context.clearCookies with a filter through the pinned Playwright and through Neo, on fresh tabs
// of an inline page (events.mjs): a name removes only the cookies of that name, and the page's
// cookieStore sees just their deletion (server/browserContext.ts `clearCookies` expires the
// matches; the others are never deleted and re-added); name and domain, and a name pattern (its
// flags kept) and path, each remove only the cookies matching every part.
//
// Neo's context is the user's own profile, so it refuses clearCookies() with no filter, where
// Playwright removes every cookie of the context: that case runs through Neo alone. Every script
// filters by names it set (`neo-cc-`), and removes what it added before it returns.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/**
 * `ours(cookies)`: the case's cookies in a stable order; `seen()`: the page's document.cookie
 * pairs; `forget(...)`: removes cookies by name.
 */
const COOKIES = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const mine = (pair) => pair.startsWith('neo-cc-')
    const ours = (cookies) => cookies.filter((cookie) => mine(cookie.name))
      .map((cookie) => cookie.name + ' ' + cookie.domain + ' ' + cookie.path).sort()
    const seen = async () => (await page.evaluate(() => document.cookie)).split('; ').filter(mine).sort()
    const forget = async (...names) => {
      for (const name of names) await page.context().clearCookies({ name })
    }
    `

const PROBE = `document.cookie.split('; ').filter((pair) => pair.startsWith('neo-cc-')).join('; ')`

const cases = [
  {
    // tests/library/browsercontext-clearcookies.spec.ts "CO-044 should remove cookies by name",
    // "CO-049 should not transiently delete non-matching cookies when filtering". A cookie added
    // after the clear marks the end of its changes: cookieStore reports them in order.
    name: 'clearCookies({ name }) removes only that cookie, and the page sees only its deletion',
    path: '/clearCookies.html',
    source: `${COOKIES}const changes = (wanted) => page.waitForFunction(
      (wanted) => wanted.every((change) => window.changes.includes(change)), wanted)
    try {
      await page.evaluate(() => {
        window.changes = []
        cookieStore.addEventListener('change', (event) => {
          for (const cookie of event.changed) window.changes.push('changed ' + cookie.name)
          for (const cookie of event.deleted) window.changes.push('deleted ' + cookie.name)
        })
      })
      await page.context().addCookies([
        { name: 'neo-cc-keep', value: '1', domain: '127.0.0.1', path: '/' },
        { name: 'neo-cc-drop', value: '2', domain: '127.0.0.1', path: '/' },
      ])
      await changes(['changed neo-cc-keep', 'changed neo-cc-drop'])
      const before = await seen()
      await page.evaluate(() => { window.changes = [] })
      await page.context().clearCookies({ name: 'neo-cc-drop' })
      await page.context().addCookies([{ name: 'neo-cc-marker', value: '3', domain: '127.0.0.1', path: '/' }])
      await changes(['changed neo-cc-marker'])
      return {
        before,
        changes: await page.evaluate(() => window.changes.filter((change) => change.includes(' neo-cc-'))),
        after: await seen(),
        stored: ours(await page.context().cookies(origin + '/')),
      }
    } finally {
      await forget('neo-cc-keep', 'neo-cc-drop', 'neo-cc-marker')
    }`,
    expected: {
      before: ['neo-cc-drop=2', 'neo-cc-keep=1'],
      changes: ['deleted neo-cc-drop', 'changed neo-cc-marker'],
      after: ['neo-cc-keep=1', 'neo-cc-marker=3'],
      stored: ['neo-cc-keep 127.0.0.1 /', 'neo-cc-marker 127.0.0.1 /'],
    },
    probe: PROBE,
    effect: '',
  },
  {
    // browsercontext-clearcookies.spec.ts "CO-048 should remove cookies by name and domain",
    // "CO-045 should remove cookies by name regex", "CO-047 should remove cookies by path".
    // Every filter names the case's cookies, so none reaches a cookie of the user's.
    name: 'clearCookies removes only the cookies matching every part of its filter',
    path: '/clearCookies.html',
    source: `${COOKIES}const otherOrigin = origin.replace('127.0.0.1', 'localhost')
    const left = async () => ours(await page.context().cookies([
      origin + '/api/v1', origin + '/api/v2', otherOrigin + '/',
    ]))
    try {
      await page.context().addCookies([
        { name: 'neo-cc-twin', value: 'here', domain: '127.0.0.1', path: '/' },
        { name: 'neo-cc-twin', value: 'there', domain: 'localhost', path: '/' },
        { name: 'neo-cc-v1', value: '1', domain: '127.0.0.1', path: '/api/v1' },
        { name: 'neo-cc-v2', value: '2', domain: '127.0.0.1', path: '/api/v2' },
        { name: 'neo-cc-root', value: '3', domain: '127.0.0.1', path: '/' },
      ])
      const added = await left()
      await page.context().clearCookies({ name: 'neo-cc-twin', domain: '127.0.0.1' })
      const byNameAndDomain = await left()
      await page.context().clearCookies({ name: /^NEO-CC-V/i, path: '/api/v1' })
      const byPatternAndPath = await left()
      return { added, byNameAndDomain, byPatternAndPath }
    } finally {
      await forget('neo-cc-twin', 'neo-cc-v1', 'neo-cc-v2', 'neo-cc-root')
    }`,
    expected: {
      added: [
        'neo-cc-root 127.0.0.1 /',
        'neo-cc-twin 127.0.0.1 /',
        'neo-cc-twin localhost /',
        'neo-cc-v1 127.0.0.1 /api/v1',
        'neo-cc-v2 127.0.0.1 /api/v2',
      ],
      byNameAndDomain: [
        'neo-cc-root 127.0.0.1 /',
        'neo-cc-twin localhost /',
        'neo-cc-v1 127.0.0.1 /api/v1',
        'neo-cc-v2 127.0.0.1 /api/v2',
      ],
      byPatternAndPath: [
        'neo-cc-root 127.0.0.1 /',
        'neo-cc-twin localhost /',
        'neo-cc-v2 127.0.0.1 /api/v2',
      ],
    },
    probe: PROBE,
    effect: '',
  },
  {
    // browsercontext-clearcookies.spec.ts "CO-042 should clear cookies", which Neo refuses:
    // with no filter, or an empty one, Playwright removes every cookie of the context, here the
    // user's whole profile. Neo only.
    name: 'Neo refuses clearCookies() with no filter and keeps every cookie',
    path: '/clearCookies.html',
    source: `${COOKIES}try {
      await page.context().addCookies([{ name: 'neo-cc-kept', value: '1', domain: '127.0.0.1', path: '/' }])
      const refused = [await settle(page.context().clearCookies()), await settle(page.context().clearCookies({}))]
      return { refused, kept: await seen() }
    } finally {
      await forget('neo-cc-kept')
    }`,
    neo: (outcome) => {
      assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
      assert.deepEqual(outcome.value, {
        refused: [{ error: 'Error' }, { error: 'Error' }],
        kept: ['neo-cc-kept=1'],
      })
    },
  },
]

test('context.clearCookies with a filter removes the same cookies through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/clearCookies.html': '<title>clearCookies</title><h1>clearCookies</h1>',
    },
    cases,
  })
})
