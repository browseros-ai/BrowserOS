/**
 * The route handlers' own logic, without a browser (route.ts): what patterns a page sends Rust,
 * how `unroute` names a handler, how `times` expires one, and a route's one answer. The browser
 * side is the route parity files' (tests/parity/route*.test.mjs).
 */
import { beforeEach, expect, test } from 'bun:test'
import type { Connection } from '../../src/connection'
import type { Request } from '../../src/network'
import { Route, RouteHandler, urlMatchesEqual } from '../../src/route'
import { TimeoutSettings } from '../../src/timeout-settings'
import { eventless } from './bridge-stub'

beforeEach(() => {
  // Rust's matcher answers a glob check; these tests only build handlers.
  ;(globalThis as Record<string, unknown>).__neo_runtime = eventless({
    call: async () => '{}',
    get(request: string) {
      const { field, matcher } = JSON.parse(request)
      if (field !== 'matchesURL') throw new Error(`unexpected getter ${field}`)
      if ('glob' in matcher && matcher.glob.includes('{{'))
        throw new Error('Invalid glob pattern')
      return 'false'
    },
  })
})

const noop = () => {}

test('tests that the patterns name each handler, and a predicate names every URL', () => {
  const glob = new RouteHandler('**/*.json', noop)
  const regex = new RouteHandler(/data\.json$/i, noop)
  expect(RouteHandler.prepareInterceptionPatterns([glob, regex])).toEqual([
    { glob: '**/*.json' },
    { regexSource: 'data\\.json$', regexFlags: 'i' },
  ])
  const predicate = new RouteHandler((url) => url.pathname === '/x', noop)
  expect(RouteHandler.prepareInterceptionPatterns([glob, predicate])).toEqual([
    { glob: '**/*' },
  ])
  expect(RouteHandler.prepareInterceptionPatterns([])).toEqual([])
})

test('tests that a bad glob throws when the route is made', () => {
  expect(() => new RouteHandler('**/{{bad', noop)).toThrow('Invalid glob')
})

test('tests that unroute names a handler by its URL as Playwright compares them', () => {
  expect(urlMatchesEqual(/a/i, /a/i)).toBe(true)
  expect(urlMatchesEqual(/a/i, /a/)).toBe(false)
  expect(urlMatchesEqual('**/a', '**/a')).toBe(true)
  const predicate = () => true
  expect(urlMatchesEqual(predicate, predicate)).toBe(true)
  expect(urlMatchesEqual(predicate, () => true)).toBe(false)
})

test('tests that times expires a handler before its last run', async () => {
  const once = new RouteHandler('**/a', noop, 1)
  expect(once.willExpire()).toBe(true)
  const twice = new RouteHandler(
    '**/a',
    (route) => (route as Route).fallback(),
    2,
  )
  expect(twice.willExpire()).toBe(false)
  const route = new Route(
    'route@T:1',
    { _applyFallbackOverrides() {} } as unknown as Request,
    {} as Connection,
    new TimeoutSettings(),
  )
  expect(await twice.handle(route)).toBe(false)
  expect(twice.willExpire()).toBe(true)
})

test('tests that a route takes one answer: a fallback hands it on', async () => {
  const route = new Route(
    'route@T:2',
    { _applyFallbackOverrides() {} } as unknown as Request,
    {} as Connection,
    new TimeoutSettings(),
  )
  const handled = route._startHandling()
  await route.fallback()
  expect(await handled).toBe(false)
  await expect(route.continue()).rejects.toThrow('Route is already handled!')
})
