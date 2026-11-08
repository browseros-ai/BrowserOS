// Adapted from pinned Playwright 26137face, tests/library/browsercontext-route.spec.ts:
// RO-165 "should intercept", RO-166 "should unroute", RO-167 "should yield to page.route",
// and RO-168 "should fall back to context.route", across two isolated contexts.
// Created-context routing is deliberately refused by Neo; Playwright supports it.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/route.html': '<title>route scope</title>',
  '/payload': 'network',
}

const SETUP = `const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
const read = (p) => p.evaluate(() => fetch('/payload').then(r => r.text()))
const created = await context.browser().newContext()
`

test('tests that context routes reach only existing and later pages of their own context', {
  skip,
}, async (t) => {
  const source = `${SETUP}
const existing = await created.newPage()
await existing.goto(origin + '/route.html')
try {
  await context.route('**/payload', route => route.fulfill({ body: 'own' }))
  const later = await created.newPage()
  await later.goto(origin + '/route.html')
  const ownLater = await context.newPage()
  await ownLater.goto(origin + '/route.html')
  const result = {
    own: await read(page), ownLater: await read(ownLater),
    existing: await read(existing), later: await read(later),
  }
  await context.unrouteAll()
  result.removed = await read(page)
  await ownLater.close()
  return result
} finally {
  await context.unrouteAll()
  await created.close()
}`
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/route.html', source))
    assert.deepEqual(reference, {
      own: 'own',
      ownLater: 'own',
      existing: 'network',
      later: 'network',
      removed: 'network',
    })
    assert.deepEqual(outcomeValue(await neo('/route.html', source)), reference)
  })
})

test('tests that a created page has local routes and never falls back to another context', {
  skip,
}, async (t) => {
  const source = `${SETUP}
try {
  await context.route('**/payload', route => route.fulfill({ body: 'own' }))
  const other = await created.newPage()
  const sibling = await created.newPage()
  await other.goto(origin + '/route.html')
  await sibling.goto(origin + '/route.html')
  await other.route('**/payload', route => route.fulfill({ body: 'page' }))
  const local = await read(other)
  const unaffected = await read(sibling)
  await other.unroute('**/payload')
  await other.route('**/payload', route => route.fallback())
  const fallback = await read(other)
  await other.unrouteAll()
  return { local, unaffected, fallback, own: await read(page) }
} finally {
  await context.unrouteAll()
  await created.close()
}`
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/route.html', source))
    assert.deepEqual(reference, {
      local: 'page',
      unaffected: 'network',
      fallback: 'network',
      own: 'own',
    })
    assert.deepEqual(outcomeValue(await neo('/route.html', source)), reference)
  })
})

test('tests that Neo explicitly refuses routing methods on a created context', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    // Playwright accepts context routing; this refusal is Neo's declared deviation.
    const accepted = `${SETUP}
try {
  await created.route('**/payload', route => route.continue())
  await created.unroute('**/payload')
  await created.unrouteAll()
  return true
} finally { await created.close() }`
    assert.equal(outcomeValue(await playwright('/route.html', accepted)), true)

    const refused = `${SETUP}
const methods = ['route', 'unroute', 'unrouteAll', 'routeFromHAR', 'routeWebSocket']
const results = []
try {
  for (const method of methods) {
    try {
      await created[method]('**/payload', () => {})
      results.push({ method, accepted: true })
    } catch (error) {
      results.push({ method, name: error.name, message: error.message })
    }
  }
  return results
} finally { await created.close() }`
    const results = outcomeValue(await neo('/route.html', refused))
    assert.ok(Array.isArray(results), JSON.stringify(results))
    for (const result of results) {
      assert.equal(result.name, 'Error', result.method)
      assert.ok(
        result.message.includes(
          `${result.method} is not served yet for a context browser.newContext created`,
        ),
        result.message,
      )
    }
    assert.equal(results.length, 5)
  })
})
