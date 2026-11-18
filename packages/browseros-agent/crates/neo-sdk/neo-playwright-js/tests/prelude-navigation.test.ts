// Navigation predicates execute in the agent realm while the native journal retains later
// events. These tests cover that handoff and the one deadline across it.
import { expect, it } from 'bun:test'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

it('retains the navigation cursor when a URL predicate rejects an earlier navigation', async () => {
  let navigation = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) =>
      name === 'page.waitForNavigation'
        ? {
            url: `https://a.example/${++navigation === 1 ? 'first' : 'target'}`,
            response: null,
            navigationSeq: 100 + navigation,
          }
        : null,
  })
  const response = await runtime.run(`return await page.waitForNavigation({
    url: url => url.pathname === '/target', timeout: 5000,
  })`)
  expect(response).toBeNull()
  const calls = runtime.sent('page.waitForNavigation')
  expect(calls).toHaveLength(2)
  expect(calls[1]?.input.after).toBe(101)
  expect(calls[1]?.input.since).toBe(calls[0]?.input.since)
})

it('bounds a pending URL predicate by the original navigation timeout', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) =>
      name === 'page.waitForNavigation'
        ? {
            url: 'https://a.example/target',
            response: null,
            navigationSeq: 101,
          }
        : null,
  })
  const error = json(
    await runtime.run(`return await page.waitForNavigation({
    url: () => new Promise(() => {}), timeout: 50,
  }).catch(error => ({ name: error.name, message: error.message }))`),
  )
  expect(error).toEqual({
    name: 'TimeoutError',
    message: 'page.waitForNavigation: Timeout 50ms exceeded.',
  })
})

// The navigation the call waited for has answered, so a frame-cache refresh after it is
// bookkeeping: bounded by the deadline, it never turns that navigation into a timeout.
it('keeps a post-navigation frame refresh inside the navigation deadline without failing the navigation', async () => {
  let frames = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'page.frames')
        return ++frames === 1 ? { frames: [] } : new Promise(() => {})
      if (name === 'page.waitForNavigation')
        return {
          url: 'https://a.example/target',
          response: {
            url: 'https://a.example/target',
            status: 200,
            statusText: 'OK',
            ok: true,
            headers: {},
          },
          navigationSeq: 101,
        }
      return null
    },
  })
  const status = await runtime.run(
    'return (await page.waitForNavigation({ timeout: 50 })).status()',
  )
  expect(status).toBe(200)
})

it('bounds a native reply that stalls after its browser-side navigation wait', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) =>
      name === 'page.waitForNavigation' ? new Promise(() => {}) : null,
  })
  const error = json(
    await runtime.run(`return await page.waitForNavigation({ timeout: 50 })
    .catch(error => ({ name: error.name, message: error.message }))`),
  )
  expect(error).toEqual({
    name: 'TimeoutError',
    message: 'page.waitForNavigation: Timeout 50ms exceeded.',
  })
})

it('returns the negotiated HTTP version read with the committed navigation', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) =>
      name === 'page.goto'
        ? {
            url: 'https://a.example/',
            response: {
              url: 'https://a.example/',
              status: 200,
              statusText: 'OK',
              httpVersion: 'HTTP/2.0',
              ok: true,
              headers: {},
            },
            frameSnapshotAttempted: true,
          }
        : null,
  })
  expect(
    await runtime.run(`const response = await page.goto('https://a.example/');
    return await response.httpVersion()`),
  ).toBe('HTTP/2.0')
})

// The predicate path: the native load-state wait answered `{}` as production's does, so the
// frame cache is refreshed after it. That refresh is bounded by the deadline but never turns the
// navigation the predicate accepted into a timeout.
it('keeps the frame refresh after a matched navigation load state from failing the navigation', async () => {
  let frames = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'page.frames')
        return ++frames === 1 ? { frames: [] } : new Promise(() => {})
      if (name === 'page.waitForNavigation')
        return {
          url: 'https://a.example/target',
          response: {
            url: 'https://a.example/target',
            status: 200,
            statusText: 'OK',
            ok: true,
            headers: {},
          },
          navigationSeq: 101,
          frameSnapshotAttempted: true,
        }
      if (name === 'page.waitForLoadState') return {}
      return null
    },
  })
  const status = await runtime.run(
    'return (await page.waitForNavigation({ url: () => true, timeout: 50 })).status()',
  )
  expect(status).toBe(200)
  expect(runtime.sent('page.waitForLoadState')).toHaveLength(1)
})
