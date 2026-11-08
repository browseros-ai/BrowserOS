// context.grantPermissions through the pinned Playwright and through Neo, on fresh tabs of an
// inline page (events.mjs), compared by what navigator.permissions.query tells the page: a grant
// for the page's origin (given as the page's URL) reads granted there and prompt on another
// origin, and every permission it leaves out reads denied there, an empty grant included; grants
// add up, and one with no origin covers every origin; an unknown permission rejects with Error
// and leaves the grants as they were.
//
// Neo's context is the user's own profile, so every script clears the permissions it granted
// before it returns (in a finally), and ends on the page it started on. The other origin is the
// fixture's localhost, another site.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/** `states()`: what navigator.permissions.query tells the page of geolocation and notifications. */
const PERMISSIONS = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const otherOrigin = origin.replace('127.0.0.1', 'localhost')
    const start = page.url()
    const states = () => page.evaluate(async () => ({
      geolocation: (await navigator.permissions.query({ name: 'geolocation' })).state,
      notifications: (await navigator.permissions.query({ name: 'notifications' })).state,
    }))
    const restore = async () => {
      await page.context().clearPermissions()
      await page.goto(start)
    }
    `

const cases = [
  {
    // tests/library/permissions.spec.ts "PE-001 should be prompt by default", "PE-002 should deny
    // permission when not listed", "PE-004 should grant geolocation permission when origin is
    // listed", "PE-005 should prompt for geolocation permission when origin is not listed".
    name: "a grant for the page's origin reads granted there, denied for what it leaves out, prompt elsewhere",
    path: '/grantPermissions.html',
    source: `${PERMISSIONS}try {
      const before = await states()
      await page.context().grantPermissions([], { origin: page.url() })
      const empty = await states()
      await page.context().grantPermissions(['geolocation'], { origin: page.url() })
      const geolocation = await states()
      await page.goto(otherOrigin + '/grantPermissions.html')
      return { before, empty, geolocation, elsewhere: await states() }
    } finally {
      await restore()
    }`,
    expected: {
      before: { geolocation: 'prompt', notifications: 'prompt' },
      empty: { geolocation: 'denied', notifications: 'denied' },
      geolocation: { geolocation: 'granted', notifications: 'denied' },
      elsewhere: { geolocation: 'prompt', notifications: 'prompt' },
    },
    probe: 'location.pathname',
    effect: '/grantPermissions.html',
  },
  {
    // permissions.spec.ts "PE-007 should accumulate when adding", "PE-009 should grant permission
    // when listed for all domains".
    name: 'grants add up, and a grant with no origin covers every origin',
    path: '/grantPermissions.html',
    source: `${PERMISSIONS}try {
      await page.context().grantPermissions(['geolocation'])
      await page.context().grantPermissions(['notifications'])
      const here = await states()
      await page.goto(otherOrigin + '/grantPermissions.html')
      return { here, elsewhere: await states() }
    } finally {
      await restore()
    }`,
    expected: {
      here: { geolocation: 'granted', notifications: 'granted' },
      elsewhere: { geolocation: 'granted', notifications: 'granted' },
    },
    probe: 'location.pathname',
    effect: '/grantPermissions.html',
  },
  {
    // permissions.spec.ts "PE-003 should fail when bad permission is given".
    name: 'an unknown permission rejects with Error and leaves the grants as they were',
    path: '/grantPermissions.html',
    source: `${PERMISSIONS}try {
      await page.context().grantPermissions(['geolocation'], { origin })
      const refused = await settle(page.context().grantPermissions(['foo'], { origin }))
      return { refused, kept: await states() }
    } finally {
      await restore()
    }`,
    expected: {
      refused: { error: 'Error' },
      kept: { geolocation: 'granted', notifications: 'denied' },
    },
    probe: 'location.pathname',
    effect: '/grantPermissions.html',
  },
]

test('context.grantPermissions grants the same permissions through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/grantPermissions.html':
        '<title>grantPermissions</title><h1>grantPermissions</h1>',
    },
    cases,
  })
})
