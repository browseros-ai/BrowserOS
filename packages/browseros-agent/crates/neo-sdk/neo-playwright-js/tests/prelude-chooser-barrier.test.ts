// The file chooser barrier (src/prelude/chooser-setup.ts) as scripts see it: a write waits for
// the interception setup of its own tab, so the chooser it opens is caught, and is never held by
// another tab whose setup has not answered.
import { describe, expect, it } from 'bun:test'
import { type Call, loadPrelude, SEED } from './support/prelude-runtime'

const never = new Promise<never>(() => {})

/** A promise the test settles when it chooses. */
function hold() {
  let release = () => {}
  const promise = new Promise<null>((resolve) => {
    release = () => resolve(null)
  })
  return { promise, release }
}

async function eventually(check: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (check()) return
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  throw new Error('the condition never held')
}

/** A few turns of the host's event loop, for an op that must not be sent. */
const turns = () => new Promise((resolve) => setTimeout(resolve, 30))

/** The tab an op's input names, where the door reads it. */
function tabOf(call: Call): unknown {
  const target = call.input.target as { page?: { tab?: unknown } } | undefined
  const page = (call.input.page ?? target?.page) as { tab?: unknown }
  return page?.tab
}

const BUSY = 1008

const MAIN_FRAME = {
  frames: [{ id: 'main', parentId: null, name: '', url: 'about:blank' }],
}

/**
 * A host whose interception setup for the busy tab answers only with `setup`, and whose pump
 * idles. The run's door opens tab 1012 for an op on the unopened page.
 */
const host =
  (setup: Promise<unknown>, frames: Promise<unknown> = Promise.resolve(null)) =>
  (call: Call) => {
    if (call.name === 'events.next') return never
    if (call.name === 'events.interceptFileChooser' && tabOf(call) === BUSY)
      return setup
    if (tabOf(call) !== null) return null
    const opened = {
      defaultPage: { tab: 1012 },
      pages: [{ tab: 1012, url: 'about:blank', title: '' }],
    }
    return call.name === 'page.frames'
      ? frames.then(() => ({ ok: MAIN_FRAME, ...opened }))
      : { ok: null, ...opened }
  }

const names = (calls: Call[]) =>
  calls.map((call) => call.name).filter((name) => name !== 'events.next')

describe('the file chooser barrier', () => {
  it("tests that a write on one tab is sent while another tab's chooser setup has not answered", async () => {
    const runtime = loadPrelude({
      seed: { ...SEED, pages: [{ tab: 1007 }, { tab: BUSY }] },
      answer: host(never),
    })
    const run = runtime.run(`
      const busy = context.pages().find((other) => other !== page)
      busy.on('filechooser', () => {})
      await page.getByText('Upload').click()
    `)
    await eventually(() => runtime.sent('locator.click').length === 1)
    await run
    expect(names(runtime.calls)).toEqual([
      'events.interceptFileChooser',
      'locator.click',
    ])
    expect(tabOf(runtime.sent('locator.click')[0] as Call)).toBe(1007)
  })

  it("tests that a write on a tab waits for that tab's chooser setup, so the chooser it opens is caught", async () => {
    const setup = hold()
    const runtime = loadPrelude({
      seed: { ...SEED, pages: [{ tab: 1007 }, { tab: BUSY }] },
      answer: host(setup.promise),
    })
    const run = runtime.run(`
      const busy = context.pages().find((other) => other !== page)
      busy.on('filechooser', () => {})
      await busy.getByText('Upload').click()
    `)
    await eventually(
      () => runtime.sent('events.interceptFileChooser').length === 1,
    )
    await turns()
    expect(runtime.sent('locator.click')).toEqual([])
    setup.release()
    await run
    expect(names(runtime.calls)).toEqual([
      'events.interceptFileChooser',
      'events.listen',
      'locator.click',
    ])
    expect(tabOf(runtime.sent('locator.click')[0] as Call)).toBe(BUSY)
  })

  it("tests that a write naming no page is sent while a tab's chooser setup has not answered", async () => {
    const runtime = loadPrelude({
      seed: { ...SEED, pages: [{ tab: 1007 }, { tab: BUSY }] },
      answer: (call) =>
        call.name === 'context.addInitScript' ? 7 : host(never)(call),
    })
    const run = runtime.run(`
      const busy = context.pages().find((other) => other !== page)
      busy.on('filechooser', () => {})
      await context.addInitScript('window.c = 1')
    `)
    await eventually(() => runtime.sent('context.addInitScript').length === 1)
    await run
  })

  it("tests that a write on the run's unopened page waits for every tab's chooser setup", async () => {
    const setup = hold()
    const runtime = loadPrelude({
      seed: { page: null, pages: [{ tab: BUSY }] },
      answer: host(setup.promise),
    })
    const run = runtime.run(`
      context.pages()[0].on('filechooser', () => {})
      await page.getByText('Upload').click()
    `)
    await eventually(
      () => runtime.sent('events.interceptFileChooser').length === 1,
    )
    await turns()
    expect(runtime.sent('locator.click')).toEqual([])
    setup.release()
    await run
    expect(tabOf(runtime.sent('locator.click')[0] as Call)).toBeNull()
  })

  it("tests that a chooser setup on the run's unopened page holds a write on a known tab until that page has its tab", async () => {
    const frames = hold()
    const runtime = loadPrelude({
      seed: { page: null, pages: [{ tab: BUSY }] },
      answer: host(Promise.resolve(null), frames.promise),
    })
    const run = runtime.run(`
      const known = context.pages()[0]
      page.on('filechooser', () => {})
      await known.getByText('Upload').click()
    `)
    await eventually(() => runtime.sent('page.frames').length === 1)
    await turns()
    expect(runtime.sent('locator.click')).toEqual([])
    frames.release()
    await run
    expect(names(runtime.calls)).toEqual([
      'page.frames',
      'events.interceptFileChooser',
      'events.listen',
      'locator.click',
    ])
    expect(tabOf(runtime.sent('events.interceptFileChooser')[0] as Call)).toBe(
      1012,
    )
  })
})
