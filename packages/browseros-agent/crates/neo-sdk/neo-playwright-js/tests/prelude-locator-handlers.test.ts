// The client half of `page.addLocatorHandler` (locator-handlers.ts): registration, the pull
// that runs each handler Rust triggers, upstream's `times` countdown and its locator equality.
import { expect, it } from 'bun:test'
import {
  type Answer,
  isRefusal,
  loadPrelude,
  SEED,
} from './support/prelude-runtime'

/** A host that numbers registrations and answers the pull when the test triggers a uid. */
function host() {
  const pulls: ((uid: number | null) => void)[] = []
  let uid = 0
  const answer: Answer = ({ name }) => {
    if (name === 'page.addLocatorHandler') return { uid: ++uid }
    if (name === 'page.locatorHandlerTriggered')
      return new Promise((resolve) =>
        pulls.push((next) => resolve({ uid: next })),
      )
    return {}
  }
  return {
    answer,
    /** Answers the pending pull with `uid`, as Rust does once an action found it visible. */
    trigger(next: number | null) {
      const pull = pulls.shift()
      if (!pull) throw new Error('no pull is pending')
      pull(next)
    },
    pending: () => pulls.length,
  }
}

/** Lets the realm's microtasks and the harness's reply turns run. */
const turns = () => new Promise((resolve) => setTimeout(resolve, 10))

it('runs a triggered handler with its own locator and resolves it once times ran out', async () => {
  const fake = host()
  const runtime = loadPrelude({ seed: SEED, answer: fake.answer })
  await runtime.run(`
    const overlay = page.getByText('Accept cookies')
    page.seen = []
    await page.addLocatorHandler(overlay, (l) => { page.seen.push(l === overlay) }, { times: 1, noWaitAfter: true })
  `)
  expect(runtime.sent('page.addLocatorHandler')[0]?.input).toMatchObject({
    call: { api: 'page.addLocatorHandler', timeoutMs: 0 },
    target: { $neo: 'locator', display: "getByText('Accept cookies')" },
    noWaitAfter: true,
  })
  await turns()
  expect(fake.pending()).toBe(1)
  fake.trigger(1)
  await turns()
  expect(await runtime.run('return page.seen')).toEqual([true])
  expect(
    runtime.sent('page.resolveLocatorHandler').map((c) => c.input),
  ).toEqual([
    {
      call: { api: 'page.resolveLocatorHandler', timeoutMs: 0 },
      uid: 1,
      remove: true,
    },
  ])
  // The pull that was already out ends with Rust's `null`, and none follows: no handler is left.
  fake.trigger(null)
  await turns()
  expect(fake.pending()).toBe(0)
})

it('refuses a locator of another page and registers nothing for times: 0', async () => {
  const fake = host()
  const runtime = loadPrelude({
    seed: { ...SEED, pages: [{ tab: 1007 }, { tab: 1008 }] },
    answer: fake.answer,
  })
  const message = await runtime.run(`
    const other = context.pages().find((p) => p !== page)
    try { await page.addLocatorHandler(other.locator('#x'), () => {}) } catch (error) { return error.message }
  `)
  expect(String(message)).toContain(
    'Locator must belong to the main frame of this page',
  )
  expect(runtime.calls.some(isRefusal)).toBe(true)
  await runtime.run(
    `await page.addLocatorHandler(page.locator('#x'), () => {}, { times: 0 })`,
  )
  // The refusal is the only call: the door recorded it under the method's name.
  expect(runtime.sent('page.addLocatorHandler').every(isRefusal)).toBe(true)
})

it('removes every registration of an equal locator on that page and no other', async () => {
  const fake = host()
  const runtime = loadPrelude({ seed: SEED, answer: fake.answer })
  await runtime.run(`
    await page.addLocatorHandler(page.getByText('A'), () => {})
    await page.addLocatorHandler(page.getByText('A'), () => {})
    await page.addLocatorHandler(page.getByText('B'), () => {})
    await page.removeLocatorHandler(page.getByText('A'))
  `)
  expect(
    runtime.sent('page.removeLocatorHandler').map((c) => c.input.uid),
  ).toEqual([1, 2])
  // A trigger of a removed handler runs nothing and still resolves, as upstream's does.
  fake.trigger(1)
  await turns()
  expect(
    runtime.sent('page.resolveLocatorHandler').map((c) => c.input),
  ).toEqual([
    { call: { api: 'page.resolveLocatorHandler', timeoutMs: 0 }, uid: 1 },
  ])
})
