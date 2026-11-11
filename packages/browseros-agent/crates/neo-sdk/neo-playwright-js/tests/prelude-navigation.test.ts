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
