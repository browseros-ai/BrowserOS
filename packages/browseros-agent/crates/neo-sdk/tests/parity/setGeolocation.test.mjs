// context.setGeolocation through the pinned Playwright and through Neo, on fresh tabs of an inline
// page (events.mjs), compared by what navigator.geolocation hands the page: getCurrentPosition
// reads the latitude, longitude and accuracy set (accuracy 0 when not given); watchPosition sees
// each new position; coordinates out of range, a negative accuracy or a missing latitude reject
// with Error and leave the position as it was (server/browserContext.ts `verifyGeolocation`).
//
// The page reads a position only with the permission: every script grants `geolocation` for
// the page's origin first, and before it returns clears the permission and the position, as
// Neo's context is the user's own profile. What the page reads once the position is cleared is
// not compared: Playwright then emulates an unavailable position.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/** `position()`: what getCurrentPosition hands the page, or its error's code. */
const GEOLOCATION = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const position = () => page.evaluate(() => new Promise((resolve) => navigator.geolocation.getCurrentPosition(
      ({ coords }) => resolve({ latitude: coords.latitude, longitude: coords.longitude, accuracy: coords.accuracy }),
      (error) => resolve({ error: error.code }),
    )))
    const restore = async () => {
      await page.context().setGeolocation(null)
      await page.context().clearPermissions()
    }
    `

const cases = [
  {
    // tests/library/geolocation.spec.ts "GE-001 should work @smoke".
    name: 'getCurrentPosition reads the position set, its accuracy 0 unless given',
    path: '/setGeolocation.html',
    source: `${GEOLOCATION}try {
      await page.context().grantPermissions(['geolocation'], { origin })
      await page.context().setGeolocation({ longitude: 10, latitude: 20, accuracy: 3 })
      const first = await position()
      await page.context().setGeolocation({ longitude: -30.5, latitude: 40.25 })
      return { first, second: await position() }
    } finally {
      await restore()
    }`,
    expected: {
      first: { latitude: 20, longitude: 10, accuracy: 3 },
      second: { latitude: 40.25, longitude: -30.5, accuracy: 0 },
    },
  },
  {
    // geolocation.spec.ts "GE-008 watchPosition should be notified".
    name: 'watchPosition sees each position set',
    path: '/setGeolocation.html',
    source: `${GEOLOCATION}const seen = (wanted) => page.waitForFunction((wanted) => window.positions.includes(wanted), wanted)
    try {
      await page.context().grantPermissions(['geolocation'], { origin })
      await page.context().setGeolocation({ latitude: 0, longitude: 0 })
      await page.evaluate(() => {
        window.positions = []
        window.watching = navigator.geolocation.watchPosition(
          ({ coords }) => window.positions.push(coords.latitude + ',' + coords.longitude),
          () => {},
        )
      })
      await seen('0,0')
      for (const [latitude, longitude] of [[0, 10], [20, 30], [40, 50]]) {
        await page.context().setGeolocation({ latitude, longitude })
        await seen(latitude + ',' + longitude)
      }
      return [...new Set(await page.evaluate(() => window.positions))]
    } finally {
      await page.evaluate(() => navigator.geolocation.clearWatch(window.watching))
      await restore()
    }`,
    expected: ['0,0', '0,10', '20,30', '40,50'],
  },
  {
    // geolocation.spec.ts "GE-002 should throw when invalid longitude", "GE-004 should throw with
    // missing latitude".
    name: 'a position out of range or missing a latitude rejects with Error and keeps the last one',
    path: '/setGeolocation.html',
    source: `${GEOLOCATION}try {
      await page.context().grantPermissions(['geolocation'], { origin })
      await page.context().setGeolocation({ longitude: 10, latitude: 10 })
      const refused = {
        longitude: await settle(page.context().setGeolocation({ longitude: 200, latitude: 10 })),
        latitude: await settle(page.context().setGeolocation({ longitude: 10, latitude: -91 })),
        accuracy: await settle(page.context().setGeolocation({ longitude: 10, latitude: 10, accuracy: -1 })),
        missingLatitude: await settle(page.context().setGeolocation({ longitude: 10 })),
      }
      return { refused, kept: await position() }
    } finally {
      await restore()
    }`,
    expected: {
      refused: {
        longitude: { error: 'Error' },
        latitude: { error: 'Error' },
        accuracy: { error: 'Error' },
        missingLatitude: { error: 'Error' },
      },
      kept: { latitude: 10, longitude: 10, accuracy: 0 },
    },
  },
]

test('context.setGeolocation hands the page the same positions through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/setGeolocation.html':
        '<title>setGeolocation</title><h1>setGeolocation</h1>',
    },
    cases,
  })
})
