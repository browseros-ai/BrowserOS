// FR-1: a run that starts on a tab reads that tab's frame tree before the agent's first
// statement, through one `page.frames` op, so `page.frames()` lists its iframes as upstream's
// does. sdk-skeleton's A4 choices: a run whose page has no tab yet reads nothing, so it opens no
// tab, and a refused read keeps the main frame alone and still starts the run. An older
// bootstrap, which hands the agent's function whatever a provider returns, gets the page at
// once, as before.
import { expect, it } from 'bun:test'
import { type Call, json, loadPrelude, SEED } from './support/prelude-runtime'

const TREE = {
  frames: [
    { id: 'main', parentId: null, name: '', url: 'https://a.example/' },
    {
      id: 'same',
      parentId: 'main',
      name: 'same',
      url: 'https://a.example/child',
    },
    { id: 'cross', parentId: 'main', name: 'cross', url: 'https://b.example/' },
  ],
}

const FRAME_NAMES = 'return page.frames().map((frame) => frame.name())'

it("tests that a run on a known tab reads its frames before the script's first statement", async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.frames' ? TREE : null),
  })
  expect(json(await runtime.run(FRAME_NAMES))).toEqual(['', 'same', 'cross'])
  expect(runtime.startCalls).toEqual([
    {
      name: 'page.frames',
      input: {
        call: { api: 'page.frames', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
      },
    },
  ])
  expect(runtime.calls).toEqual([])
})

it('tests that a run whose page has no tab yet reads nothing and opens no tab', async () => {
  const runtime = loadPrelude({
    answer: (call) => (call.name === 'page.frames' ? TREE : null),
  })
  expect(json(await runtime.run(FRAME_NAMES))).toEqual([''])
  expect([...runtime.startCalls, ...runtime.calls]).toEqual([])
})

it('tests that a refused frames read keeps the main frame and starts the run', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'page.frames'
        ? {
            err: {
              code: 'owner_has_control',
              owner: 'host',
              delivery: 'not_sent',
              retryable: true,
              operationId: 'run.1',
              timeout: false,
              message: 'page.frames: the owner has control of this tab',
            },
          }
        : 'Title',
  })
  const outcome = json(
    await runtime.run(
      'return { frames: page.frames().length, title: await page.title() }',
    ),
  )
  expect(outcome).toEqual({ frames: 1, title: 'Title' })
  expect(runtime.startCalls.map((call) => call.name)).toEqual(['page.frames'])
  expect(runtime.calls.map((call) => call.name)).toEqual(['page.title'])
})

it('tests that an older bootstrap gets the run page at once and reads no frames', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    withoutRefuse: true,
    answer: (call) => (call.name === 'page.frames' ? TREE : null),
  })
  expect(json(await runtime.run(FRAME_NAMES))).toEqual([''])
  expect([...runtime.startCalls, ...runtime.calls]).toEqual([])
})

// FR-1's waits: page and frame waitForTimeout end with one `page.frames` read per open working
// page that has a tab (the run's page, pages first seen after the run started, and listed tabs
// the script ran an op on), so a frame attached, moved or removed during the wait shows in
// `frames()`, as upstream's wait, a server round trip, shows it.

/** Each `page.frames` read the run sent after it started: the page it named, and the API. */
const reads = (calls: Call[]) =>
  calls
    .filter((call) => call.name === 'page.frames')
    .map((call) => [
      Reflect.get(Object(call.input.page), 'tab'),
      Reflect.get(Object(call.input.call), 'api'),
    ])

it('tests that waitForTimeout reads the frames of each working page once, and of no other', async () => {
  const runtime = loadPrelude({
    seed: {
      ...SEED,
      pages: [
        { tab: 1007, url: 'https://a.example/', title: 'A' },
        { tab: 1009, url: 'https://mail.example/', title: 'Mail' },
      ],
    },
    // The title's reply names a tab the run had not seen, as one the page opened would be.
    answer: (call) =>
      call.name === 'page.frames'
        ? TREE
        : {
            ok: 'Mail',
            pages: [{ tab: 1011, url: 'https://b.example/', title: 'B' }],
          },
  })
  await runtime.run(`
    await page.waitForTimeout(1)
    await context.pages()[1].title()
    await page.mainFrame().waitForTimeout(1)
  `)
  expect(reads(runtime.startCalls)).toEqual([[1007, 'page.frames']])
  expect(reads(runtime.calls)).toEqual([
    [1007, 'page.waitForTimeout'],
    [1007, 'frame.waitForTimeout'],
    [1009, 'frame.waitForTimeout'],
    [1011, 'frame.waitForTimeout'],
  ])
})

/** A run whose `page.goto` answers only once `land()` is called, as a stalled server does. */
function stalledGoto() {
  let land: () => void = () => {}
  const landed = new Promise<void>((resolve) => {
    land = resolve
  })
  const runtime = loadPrelude({
    seed: SEED,
    answer: async (call) => {
      if (call.name === 'page.frames') return TREE
      if (call.name !== 'page.goto') return null
      await landed
      return { response: null, url: 'https://a.example/next' }
    },
  })
  return { runtime, land }
}

it('tests that a wait skips a page whose navigation is in flight, which Chromium would not answer, and reads it again once it ends', async () => {
  const { runtime, land } = stalledGoto()
  await runtime.run(`
    globalThis.navigation = page.goto('https://a.example/next')
    await page.waitForTimeout(1)
  `)
  expect(reads(runtime.calls)).toEqual([])
  land()
  await runtime.run('await globalThis.navigation; await page.waitForTimeout(1)')
  expect(reads(runtime.calls).map(([, api]) => api)).toContain(
    'page.waitForTimeout',
  )
})

it('tests that a wait reads nothing for a lazy page or a listed tab the script never touched', async () => {
  const runtime = loadPrelude({
    seed: {
      page: null,
      pages: [{ tab: 1009, url: 'https://mail.example/', title: 'Mail' }],
    },
    answer: (call) => (call.name === 'page.frames' ? TREE : null),
  })
  const frames = json(
    await runtime.run(
      'await page.waitForTimeout(1); return page.frames().length',
    ),
  )
  expect(frames).toEqual(1)
  expect([...runtime.startCalls, ...runtime.calls]).toEqual([])
})

it('tests that a frame missing from the tree a wait reads is detached', async () => {
  const removed = { frames: TREE.frames.filter((frame) => frame.id !== 'same') }
  let read = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'page.frames' ? (read++ === 0 ? TREE : removed) : null,
  })
  const outcome = json(
    await runtime.run(`
      const same = page.frame('same')
      await page.waitForTimeout(1)
      return {
        detached: same.isDetached(),
        byName: page.frame('same'),
        names: page.frames().map((frame) => frame.name()),
      }
    `),
  )
  expect(outcome).toEqual({
    detached: true,
    byName: null,
    names: ['', 'cross'],
  })
})

it.each([false, true])(
  'tests that a native removal is fenced even when its reply closes the page (%s)',
  async (closed) => {
    const nestedTree = {
      frames: [
        ...TREE.frames,
        {
          id: 'nested',
          parentId: 'same',
          name: 'nested',
          url: 'https://a.example/nested',
        },
      ],
    }
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => {
        if (call.name === 'page.frames') return nestedTree
        return {
          err: {
            owner: 'neo',
            code: 'target_closed',
            delivery: 'not_sent',
            message: 'frame.waitForURL: Navigating frame was detached!',
          },
          pages: [{ tab: 1007, closed }],
          observed: { seq: 2, events: 10, payloadCurrent: true },
        }
      },
    })
    expect(
      json(
        await runtime.run(`
        const same = page.frame('same')
        const nested = page.frame('nested')
        try { await same.waitForURL('**/never') } catch {}
        // An older pull may deliver either frame after the whole subtree was retired.
        page._eventFrame(9, 'framenavigated', {
          id: 'same', parentId: 'main', name: 'same', url: 'https://a.example/old',
        })
        page._eventFrame(9, 'framenavigated', {
          id: 'nested', parentId: 'same', name: 'nested', url: 'https://a.example/old-child',
        })
        return { detached: same.isDetached(), nestedDetached: nested.isDetached(), names: page.frames().map(f => f.name()) }
      `),
      ),
    ).toEqual({ detached: true, nestedDetached: true, names: ['', 'cross'] })
  },
)

it('tests that a URL wait timeout does not detach its live frame', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'page.frames'
        ? TREE
        : {
            err: {
              owner: 'website',
              code: 'wait_timeout',
              delivery: 'not_sent',
              message: 'frame.waitForURL: Timeout 10ms exceeded.',
            },
          },
  })
  expect(
    json(
      await runtime.run(`
        const same = page.frame('same')
        try { await same.waitForURL('**/never', { timeout: 10 }) } catch {}
        return { detached: same.isDetached(), same: page.frame('same') === same }
      `),
    ),
  ).toEqual({ detached: false, same: true })
})

for (const predicate of [false, true]) {
  for (const removed of [false, true]) {
    it(`tests that an ambiguous frame wait error confirms the tree (predicate=${predicate}, removed=${removed})`, async () => {
      let reads = 0
      const runtime = loadPrelude({
        seed: SEED,
        answer: (call) => {
          if (call.name === 'page.frames') {
            if (reads++ === 0) return TREE
            return {
              ok: removed
                ? { frames: TREE.frames.filter((f) => f.id !== 'same') }
                : TREE,
              pages: [{ tab: 1007, closed: false }],
              observed: { seq: 3, events: 10, payloadCurrent: true },
            }
          }
          return {
            err: {
              owner: 'neo',
              code: 'target_closed',
              delivery: 'not_sent',
              message: 'frame.waitForURL: Frame was detached',
            },
            pages: [{ tab: 1007, closed: false }],
            observed: { seq: 2, events: 9, payloadCurrent: true },
          }
        },
      })
      expect(
        json(
          await runtime.run(`
        const same = page.frame('same')
        try { await same.waitForURL(${predicate ? '() => true' : "'**/never'"}) } catch {}
        return { detached: same.isDetached(), names: page.frames().map(f => f.name()) }
      `),
        ),
      ).toEqual({
        detached: removed,
        names: removed ? ['', 'cross'] : ['', 'same', 'cross'],
      })
      expect(reads).toBe(2)
      expect(
        runtime.calls.find((call) => call.name === 'page.frames')?.input.call,
      ).toEqual({ api: 'frame.waitForURL', timeoutMs: 1000 })
    })
  }
}

it('tests that a user URL predicate cannot report native frame removal', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.frames' ? TREE : null),
  })
  expect(
    json(
      await runtime.run(`
    const same = page.frame('same')
    try {
      await same.waitForURL(() => {
        throw Object.assign(new Error('frame.waitForURL: Navigating frame was detached!'), { code: 'target_closed' })
      })
    } catch {}
    return same.isDetached()
  `),
    ),
  ).toBe(false)
})
