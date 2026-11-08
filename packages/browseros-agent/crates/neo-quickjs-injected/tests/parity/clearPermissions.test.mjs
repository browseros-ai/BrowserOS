// context.clearPermissions through the pinned Playwright and through Neo, on fresh tabs of an
// inline page (events.mjs), compared by what navigator.permissions tells the page: a granted
// permission reads prompt again; a grant after the clear starts from nothing, so a permission
// granted before it reads denied (server/browserContext.ts `clearPermissions` also forgets the
// grants it adds up); the page's PermissionStatus hears every change, the clear's included.
//
// Neo's context is the user's own profile, so every script clears again in a finally.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/** `states()`: what navigator.permissions.query tells the page of geolocation and notifications. */
const PERMISSIONS = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const states = () => page.evaluate(async () => ({
      geolocation: (await navigator.permissions.query({ name: 'geolocation' })).state,
      notifications: (await navigator.permissions.query({ name: 'notifications' })).state,
    }))
    `

const cases = [
  {
    // tests/library/permissions.spec.ts "PE-011 should reset permissions".
    name: 'clearPermissions turns a granted permission back to prompt',
    path: '/clearPermissions.html',
    source: `${PERMISSIONS}try {
      await page.context().grantPermissions(['geolocation'], { origin })
      const granted = await states()
      const cleared = await settle(page.context().clearPermissions())
      return { granted, cleared, after: await states() }
    } finally {
      await page.context().clearPermissions()
    }`,
    expected: {
      granted: { geolocation: 'granted', notifications: 'denied' },
      cleared: 'undefined',
      after: { geolocation: 'prompt', notifications: 'prompt' },
    },
  },
  {
    // permissions.spec.ts "PE-008 should clear permissions".
    name: 'a grant after clearPermissions starts from nothing',
    path: '/clearPermissions.html',
    source: `${PERMISSIONS}try {
      await page.context().grantPermissions(['geolocation'])
      await page.context().clearPermissions()
      await page.context().grantPermissions(['notifications'])
      return await states()
    } finally {
      await page.context().clearPermissions()
    }`,
    expected: { geolocation: 'denied', notifications: 'granted' },
  },
  {
    // permissions.spec.ts "PE-012 should trigger permission onchange".
    name: "the page's PermissionStatus hears each grant and the clear",
    path: '/clearPermissions.html',
    source: `${PERMISSIONS}const heard = (count) => page.waitForFunction((count) => window.changes.length >= count, count)
    try {
      await page.evaluate(async () => {
        window.geolocationStatus = await navigator.permissions.query({ name: 'geolocation' })
        window.changes = [window.geolocationStatus.state]
        window.geolocationStatus.onchange = () => window.changes.push(window.geolocationStatus.state)
      })
      await page.context().grantPermissions([], { origin })
      await heard(2)
      await page.context().grantPermissions(['geolocation'], { origin })
      await heard(3)
      await page.context().clearPermissions()
      await heard(4)
      return await page.evaluate(() => window.changes)
    } finally {
      await page.context().clearPermissions()
    }`,
    expected: ['prompt', 'denied', 'granted', 'prompt'],
  },
]

test('context.clearPermissions clears the same permissions through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/clearPermissions.html':
        '<title>clearPermissions</title><h1>clearPermissions</h1>',
    },
    cases,
  })
})
