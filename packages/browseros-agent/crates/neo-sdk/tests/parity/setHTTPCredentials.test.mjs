// context.setHTTPCredentials through the pinned Playwright and through Neo, on fresh tabs of an
// inline page (events.mjs), against a page behind HTTP Basic auth: with no credentials its
// navigation fails, with them it loads (200, its body), and null takes them away; a credential
// with an origin answers only that origin, the first one matching a URL answers it, and one with
// no origin matches every origin (server/browserContext.ts `findMatchingHttpCredentials`); a
// wrong one reaches the 401 page.
//
// With no credential to give, the navigation rejects with Error, as Playwright's suite expects
// on Chromium other than headless shell (`failsOn401`: net::ERR_INVALID_AUTH_CREDENTIALS), and
// the browser then commits its error page; each visit waits for the document it commits, so the
// next navigation starts after it.
//
// The browser keeps every answer it gave a server and offers it again, for that realm and below
// that path, so each visit asks in a realm of its own that is also the password: an answer from
// an earlier visit, or Playwright's turn, never answers for another. Neo's context is the user's
// own profile, so every script sets the credentials back to null (in a finally) and ends on the
// page it started on.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/**
 * `realm()`: a realm no visit has asked in; `visit(at, realm)`: how the navigation to the auth
 * page there settled (its status), once its document, or the error page, has committed.
 */
const AUTH = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const otherOrigin = origin.replace('127.0.0.1', 'localhost')
    const start = page.url()
    const realm = () => 'r' + Date.now().toString(36) + Math.random().toString(36).slice(2)
    const visit = async (at, realm) => {
      const [, status] = await Promise.all([
        page.waitForEvent('framenavigated'),
        settle(page.goto(at + '/auth?realm=' + realm).then((response) => response.status())),
      ])
      return status
    }
    const restore = async () => {
      await page.context().setHTTPCredentials(null)
      await page.goto(start)
    }
    `

/** Lets in `user` with the realm named by `?realm=` as the password. */
function basicAuth(request, response) {
  const realm = new URL(request.url, 'http://fixture').searchParams.get('realm')
  const expected = `Basic ${Buffer.from(`user:${realm}`).toString('base64')}`
  response.setHeader('content-type', 'text/html; charset=utf-8')
  if (request.headers.authorization !== expected) {
    response.statusCode = 401
    response.setHeader('www-authenticate', `Basic realm="${realm}"`)
    response.end('<title>denied</title><h1>denied</h1>')
    return
  }
  response.end('<title>granted</title><h1>granted</h1>')
}

/**
 * Neo's side of every case (neo-runtime session_state.rs `set_http_credentials`, Deviation): Neo
 * refuses setHTTPCredentials with an Error until its routes answer a server's credentials
 * challenge (pw-net-network B3); Playwright's stays pinned by `expected`.
 */
const refusesCredentials = (outcome) => {
  assert.equal(outcome.status, 'error', JSON.stringify(outcome))
  assert.equal(outcome.error.name, 'Error')
}

const cases = [
  {
    // tests/library/browsercontext-credentials.spec.ts "AU-001 should fail without credentials",
    // "AU-002 should work with setHTTPCredentials", "AU-006 should return resource body".
    name: 'a page behind Basic auth fails with no credentials, loads with them, and fails again after null',
    path: '/setHTTPCredentials.html',
    source: `${AUTH}const first = realm()
    try {
      const without = await visit(origin, first)
      await page.context().setHTTPCredentials({ username: 'user', password: first })
      const reloaded = await page.reload()
      const withCredentials = { status: reloaded.status(), title: await page.title() }
      await page.context().setHTTPCredentials(null)
      return { without, withCredentials, afterNull: await visit(origin, realm()) }
    } finally {
      await restore()
    }`,
    expected: {
      without: { error: 'Error' },
      withCredentials: { status: 200, title: 'granted' },
      afterNull: { error: 'Error' },
    },
    deviation: refusesCredentials,
    probe: 'location.pathname',
    effect: '/setHTTPCredentials.html',
  },
  {
    // browsercontext-credentials.spec.ts "AU-003 should work with setHTTPCredentials and multiple
    // credentials": the other origin's wrong credential reaches the 401.
    name: 'a credential with an origin answers only that origin',
    path: '/setHTTPCredentials.html',
    source: `${AUTH}const answer = realm()
    try {
      await page.context().setHTTPCredentials([
        { username: 'user', password: answer, origin },
        { username: 'user', password: 'wrong', origin: otherOrigin },
      ])
      return { here: await visit(origin, answer), elsewhere: await visit(otherOrigin, answer) }
    } finally {
      await restore()
    }`,
    expected: { here: { value: 200 }, elsewhere: { value: 401 } },
    deviation: refusesCredentials,
    probe: 'location.pathname',
    effect: '/setHTTPCredentials.html',
  },
  {
    // browsercontext-credentials.spec.ts "AU-009 should fall back to credentials without origin",
    // "AU-010 should use the first matching credential", through setHTTPCredentials.
    name: 'the first credential matching the URL answers it, one with no origin matching every origin',
    path: '/setHTTPCredentials.html',
    source: `${AUTH}try {
      const fallback = realm()
      await page.context().setHTTPCredentials([
        { username: 'user', password: 'wrong', origin: otherOrigin },
        { username: 'user', password: fallback },
      ])
      const noOrigin = await visit(origin, fallback)
      const first = realm()
      await page.context().setHTTPCredentials([
        { username: 'user', password: 'wrong' },
        { username: 'user', password: first, origin },
      ])
      return { noOrigin, firstMatching: await visit(origin, first) }
    } finally {
      await restore()
    }`,
    expected: { noOrigin: { value: 200 }, firstMatching: { value: 401 } },
    deviation: refusesCredentials,
    probe: 'location.pathname',
    effect: '/setHTTPCredentials.html',
  },
]

test('context.setHTTPCredentials answers the same challenges through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/setHTTPCredentials.html':
        '<title>setHTTPCredentials</title><h1>setHTTPCredentials</h1>',
      '/auth': basicAuth,
    },
    cases,
  })
})
