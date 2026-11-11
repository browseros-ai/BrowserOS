// The main-world asset as the evaluate guard calls it: one function expression, compiled in a
// realm with nothing of Neo in it, called with its mode, the script and the arguments' JSON text.
// The guard's own live tests run it in real pages; these pin the contract between the two halves.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createContext, runInContext } from 'node:vm'

const asset = readFileSync(
  new URL('../../neo-playwright/assets/main-world.js', import.meta.url),
  'utf8',
)

type MainWorld = (
  mode: 'evaluate' | 'poller',
  isFunction: boolean | undefined,
  expression: string,
  argCount: number,
  ...argTexts: string[]
) => unknown

type Poll = (() => unknown) & { first?: unknown; last?: unknown }

/** The asset compiled the way CDP compiles a declaration: inside parentheses. */
function mainWorld(): { call: MainWorld; realm: object } {
  const realm = createContext({})
  return { call: runInContext(`(${asset})`, realm) as MainWorld, realm }
}

it('tests that a result leaves as the text of upstream serialized format', () => {
  const { call } = mainWorld()
  const text = call(
    'evaluate',
    true,
    '() => [undefined, null, NaN, -0, 10n, new Date(0), /a/g, "s", { k: 1 }]',
    0,
  )
  expect(JSON.parse(String(text))).toEqual({
    a: [
      { v: 'undefined' },
      { v: 'null' },
      { v: 'NaN' },
      { v: '-0' },
      { bi: '10' },
      { d: '1970-01-01T00:00:00.000Z' },
      { r: { p: 'a', f: 'g' } },
      's',
      { o: [{ k: 'k', v: 1 }], id: 2 },
    ],
    id: 1,
  })
})

it('tests that an argument arrives as the value its serialized text describes', () => {
  const { call } = mainWorld()
  const arg = JSON.stringify({ o: [{ k: 'n', v: 41 }], id: 1 })
  expect(call('evaluate', true, '(x) => x.n + 1', 1, arg)).toBe('42')
  expect(call('evaluate', false, '6 * 7', 0)).toBe('42')
})

it('tests that undefined and a dropped value are no text, and a promise is awaited first', async () => {
  const { call } = mainWorld()
  // upstream's jsonValue answers a top-level undefined with no value at all.
  expect(call('evaluate', true, '() => undefined', 0)).toBeUndefined()
  expect(call('evaluate', true, '() => () => 1', 0)).toBeUndefined()
  expect(await call('evaluate', true, 'async () => [1]', 0)).toBe(
    JSON.stringify({ a: [1], id: 1 }),
  )
})

it('tests that the poller keeps its argument and its evaluated predicate across polls', async () => {
  const { call } = mainWorld()
  const poll = call(
    'poller',
    true,
    '(state) => ++state.n >= 3 && state',
    1,
    JSON.stringify({ o: [{ k: 'n', v: 0 }], id: 1 }),
  ) as Poll
  expect(poll.first).toEqual([false])
  expect(poll()).toEqual([false])
  expect(await poll()).toEqual([
    true,
    JSON.stringify({ o: [{ k: 'n', v: 3 }], id: 1 }),
  ])
  expect(poll.last).toEqual({ n: 3 })
})

it('tests that a string predicate is evaluated again on every poll', async () => {
  const { call, realm } = mainWorld()
  const poll = call(
    'poller',
    false,
    'globalThis.ready',
    1,
    '{"v":"undefined"}',
  ) as Poll
  expect(poll.first).toEqual([false])
  runInContext('globalThis.ready = "yes"', realm)
  expect(await poll()).toEqual([true, '"yes"'])
})

it('tests that a truthy thenable settles the wait with whatever it resolves to', async () => {
  const { call } = mainWorld()
  const poll = call(
    'poller',
    true,
    'async () => 0',
    1,
    '{"v":"undefined"}',
  ) as Poll
  expect(await poll.first).toEqual([true, '0'])
  const rejecting = call(
    'poller',
    true,
    'async () => { throw new Error("no") }',
    1,
    '{"v":"undefined"}',
  ) as Poll
  await expect(Promise.resolve(rejecting.first)).rejects.toThrow('no')
})

it('tests that a predicate throwing on its first poll fails the call that builds it', () => {
  const { call } = mainWorld()
  expect(() =>
    call(
      'poller',
      true,
      '() => { throw new Error("at once") }',
      1,
      '{"v":"undefined"}',
    ),
  ).toThrow('at once')
})

// A page owns the globals JSON text could be spelled with: Playwright's own specs (page-evaluate
// EV-069, EV-088, EV-095) replace them, and upstream's evaluate still works, because CDP carries
// its values by value. Neo's text must not depend on them either.
it('tests that a page without JSON still sends and receives its values', () => {
  const { call, realm } = mainWorld()
  runInContext('JSON.stringify = null; globalThis.JSON = null', realm)
  const arg = JSON.stringify({ o: [{ k: 'abc', v: 122 }], id: 1 })
  expect(call('evaluate', true, '(x) => ({ abc: x.abc + 1 })', 1, arg)).toBe(
    JSON.stringify({ o: [{ k: 'abc', v: 123 }], id: 1 }),
  )
})

it("tests that an array's toJSON on the page does not change the text", () => {
  const { call } = mainWorld()
  const text = call(
    'evaluate',
    true,
    '() => { Array.prototype.toJSON = () => "busted"; return [1, 2, 3] }',
    0,
  )
  expect(text).toBe(JSON.stringify({ a: [1, 2, 3], id: 1 }))
})

it('tests that busted array methods on the page do not break the transport', async () => {
  const { call, realm } = mainWorld()
  runInContext('Array.prototype.map = null; Array.prototype.push = null', realm)
  expect(call('evaluate', false, '1+2', 1, '{"v":"undefined"}')).toBe('3')
  const poll = call('poller', false, '1+2', 1, '{"v":"undefined"}') as Poll
  expect(await poll.first).toEqual([true, '3'])
})
