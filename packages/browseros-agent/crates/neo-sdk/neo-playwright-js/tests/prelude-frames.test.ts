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
  // Tried once more before the first statement, then again once the script's next operation
  // is answered: a refusal stays the tree's state until a read succeeds.
  expect(runtime.startCalls.map((call) => call.name)).toEqual([
    'page.frames',
    'page.frames',
  ])
  expect(runtime.calls.map((call) => call.name)).toEqual([
    'page.title',
    'page.frames',
  ])
})

it('tests that an omitted metadata tree preserves retained Frame objects', async () => {
  let reads = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'page.frames' ? (reads++ === 0 ? TREE : {}) : null,
  })
  const result = json(
    await runtime.run(`
      const saved = page.frame('cross')
      const frames = page.frames()
      await page.waitForTimeout(1)
      return {
        identity: saved === page.frame('cross'),
        attached: !saved.isDetached(),
        url: saved.url(),
        frames: page.frames().every((frame, index) => frame === frames[index]),
        parent: saved.parentFrame() === page.mainFrame(),
      }
    `),
  )
  expect(result).toEqual({
    identity: true,
    attached: true,
    url: 'https://b.example/',
    frames: true,
    parent: true,
  })
  expect(reads).toBe(2)
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
  expect(reads(runtime.startCalls)).toEqual([
    [1007, 'page.frames'],
    [1009, 'page.frames'],
  ])
  expect(reads(runtime.calls)).toEqual([
    [1007, 'page.waitForTimeout'],
    // The title's reply named 1011: a page first seen mid-run has its tree read at once.
    [1011, 'page.frames'],
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

it('tests that a run with no tab of its own reads each listed tab once at its start, and a wait reads none the script left alone', async () => {
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
  expect(reads(runtime.startCalls)).toEqual([[1009, 'page.frames']])
  expect(runtime.calls).toEqual([])
})

// Every page a script can reach lists its whole frame tree from the script's first statement:
// the init's page facts carry no tree, so before the run starts the prelude reads one for every
// listed tab, not only for the run's own page. Another task's tab, taken from context.pages(),
// is the shape a host's observer reads.

/** A run on tab 1007 that lists 1009, another task's tab with a same-origin and a cross-site child. */
const OBSERVER = {
  page: { $neo: 'page', tab: 1007 },
  pages: [
    { tab: 1007, url: 'https://agent.example/', title: 'Agent', closed: false },
    { tab: 1009, url: 'https://a.example/', title: 'Other', closed: false },
  ],
  observed: { seq: 0, events: 0 },
}

/** The tab a `page.frames` call names. */
const tabOf = (call: Call) => Reflect.get(Object(call.input.page), 'tab')

const OTHER = 'context.pages().find((q) => q.toJSON().tab === 1009)'

it("tests that another task's tab lists its cross-site and same-origin frames at the script's first statement", async () => {
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => (call.name === 'page.frames' ? TREE : null),
  })
  const outcome = json(
    await runtime.run(`
      const p = ${OTHER}
      return { same: p === page, names: p.frames().map((f) => f.name()), urls: p.frames().map((f) => f.url()) }
    `),
  )
  expect(outcome).toEqual({
    same: false,
    names: ['', 'same', 'cross'],
    urls: [
      'https://a.example/',
      'https://a.example/child',
      'https://b.example/',
    ],
  })
  // A listed tab's read is bounded, as a tab another run is navigating answers nothing.
  expect(
    runtime.startCalls
      .filter((call) => call.name === 'page.frames')
      .map((call) => [
        tabOf(call),
        Reflect.get(Object(call.input.call), 'timeoutMs'),
      ]),
  ).toEqual([
    [1007, 0],
    [1009, 1000],
  ])
})

it("tests that a frame added to a tab the script only reads shows after the script's next wait", async () => {
  let reads1009 = 0
  const added = {
    frames: [
      ...TREE.frames,
      { id: 'late', parentId: 'main', name: 'late', url: 'https://c.example/' },
    ],
  }
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name !== 'page.frames') return null
      if (tabOf(call) !== 1009) return TREE
      return reads1009++ === 0 ? TREE : added
    },
  })
  const counts = json(
    await runtime.run(`
      const p = ${OTHER}
      const before = p.frames().length
      await page.waitForTimeout(1)
      return [before, p.frames().length]
    `),
  )
  expect(counts).toEqual([3, 4])
})

// A tab the script has not used is read in the background (context.ts, `_readInitialFrames`). One
// whose read failed is not the script using it: a wait leaves it alone, and only the script's own
// use reads it again, holding the script for that read as for any page it uses.
it('tests that a listed tab whose start read failed is read again only once the script uses it', async () => {
  let reads1009 = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: async (call) => {
      if (call.name === 'page.title') return 'Other'
      if (call.name !== 'page.frames') return null
      if (tabOf(call) !== 1009) return TREE
      // The start read fails, and is not retried for a tab the script has not used; the read
      // after the script's own op answers late.
      if (reads1009++ === 0)
        return {
          err: {
            code: 'timeout',
            owner: 'host',
            delivery: 'unknown',
            retryable: true,
            operationId: 'run.2',
            timeout: true,
            message: 'page.frames: Timeout 1000ms exceeded',
          },
        }
      await new Promise((resolve) => setTimeout(resolve, 30))
      return TREE
    },
  })
  const counts = json(
    await runtime.run(`
      await page.waitForTimeout(150)
      const p = ${OTHER}
      const before = p.frames().length
      await p.title()
      return [before, p.frames().length]
    `),
  )
  expect(counts).toEqual([1, 3])
  expect(reads(runtime.calls)).toEqual([
    [1007, 'page.waitForTimeout'],
    [1009, 'page.frames'],
  ])
})

// The host's init lists every open tab with its workers, and the prelude subscribes to each
// one's worker state (`observeWorkers`), so Neo's own `events.listen` reaches every listed tab.
// That upkeep is not the script using a tab: a wait still reads only the pages the script used.
// Each read is an op on its tab, which a view following the run's task would go to.

/** The init of a run on tab 1007 that lists the launch tab 1001, 1007 and 1009, with workers. */
const LISTED = {
  page: { $neo: 'page', tab: 1007 },
  pages: [1001, 1007, 1009].map((tab) => ({
    tab,
    url: `https://${tab}.example/`,
    title: String(tab),
    closed: false,
    workers: [],
  })),
  observed: { seq: 0, events: 0 },
}

/** `page.frames` reads the tree; `events.next` never answers, so the pump idles. */
const listedAnswer = (call: Call) => {
  if (call.name === 'page.frames') return TREE
  if (call.name === 'events.next') return new Promise(() => {})
  return null
}

/** The tab and event of every `events.listen` the run sent, its start's included. */
const listens = (runtime: { startCalls: Call[]; calls: Call[] }) =>
  [...runtime.startCalls, ...runtime.calls]
    .filter((call) => call.name === 'events.listen')
    .map((call) => [
      Reflect.get(Object(call.input.page), 'tab'),
      call.input.event,
    ])

it("tests that Neo's own listener on every listed tab leaves a wait reading only the run's page", async () => {
  const runtime = loadPrelude({ seed: LISTED, answer: listedAnswer })
  await runtime.run(`
    await page.waitForTimeout(1)
    await page.waitForTimeout(1)
  `)
  expect(listens(runtime)).toEqual([
    [1001, 'workerstate'],
    [1007, 'workerstate'],
    [1009, 'workerstate'],
  ])
  expect(reads(runtime.calls)).toEqual([
    [1007, 'page.waitForTimeout'],
    [1007, 'page.waitForTimeout'],
  ])
})

/** The door's answer to an interception it could not set up. */
const REFUSED = {
  err: {
    code: 'owner_has_control',
    owner: 'host',
    delivery: 'not_sent',
    retryable: true,
    operationId: 'run.1',
    timeout: false,
    message: 'page.filechooser: the owner has control of this tab',
  },
}

// A file chooser listener's interception is set up before its `events.listen` goes out, so the
// page is working from the script's call on, whether that setup is answered, still pending as
// the wait ends, or refused.
it("tests that the script's own listener on another listed tab makes that tab working", async () => {
  const cases: [string, unknown][] = [
    ['framenavigated', null],
    ['filechooser', null],
    ['filechooser', new Promise(() => {})],
    ['filechooser', REFUSED],
  ]
  for (const [event, interception] of cases) {
    const runtime = loadPrelude({
      seed: LISTED,
      answer: (call) =>
        call.name === 'events.interceptFileChooser'
          ? interception
          : listedAnswer(call),
    })
    await runtime.run(`
      context.pages().find((other) => other.toJSON().tab === 1009).on('${event}', () => {})
      await page.waitForTimeout(1)
      await page.waitForTimeout(1)
    `)
    expect(reads(runtime.calls)).toEqual([
      [1007, 'page.waitForTimeout'],
      [1009, 'page.waitForTimeout'],
      [1007, 'page.waitForTimeout'],
      [1009, 'page.waitForTimeout'],
    ])
  }
})

// The run's own page can come back short at the start: the native read could not read an
// out-of-process child (`incomplete`), ran out of its own time (no tree), or was refused. The
// first-host shape: a later run focuses and types through refs into the frame, then walks
// frames() with no wait. The first awaited operation reads the tree again before it resolves.
const MAIN_ONLY = { frames: [TREE.frames[0]] }
/** TREE with one more cross-site frame, as a tab looks once a frame was added to it. */
const ADDED = {
  frames: [
    ...TREE.frames,
    { id: 'late', parentId: 'main', name: 'late', url: 'https://c.example/' },
  ],
}
const REFUSED_READ = {
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

it("tests that the run's own page whose start read came back short is read once more before the script's first statement", async () => {
  for (const short of [{ ...MAIN_ONLY, incomplete: true }, {}]) {
    let read = 0
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'page.frames' ? (read++ === 0 ? short : TREE) : 'ok',
    })
    const names = json(
      await runtime.run('return page.frames().map((f) => f.name())'),
    )
    expect(names).toEqual(['', 'same', 'cross'])
  }
})

it("tests that the run's own page whose start reads were refused has its whole tree after the script's first awaited operation", async () => {
  let read = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'page.frames') return read++ < 2 ? REFUSED_READ : TREE
      return 'ok'
    },
  })
  const counts = json(
    await runtime.run(`
      const atStart = page.frames().length
      await page.keyboard.type('4')
      return [atStart, page.frames().map((f) => f.name())]
    `),
  )
  expect(counts).toEqual([1, ['', 'same', 'cross']])
})

it("tests that a frame added to another task's tab mid-run shows after the script's next awaited operation, with no wait", async () => {
  let reads = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1009 && reads++ > 0 ? ADDED : TREE
      // The reply of the script's op on its own page carries every tab's frame-tree version.
      return { ok: 'Agent', frames: [{ tab: 1009, version: 3 }] }
    },
  })
  const counts = json(
    await runtime.run(`
      const p = ${OTHER}
      const before = p.frames().length
      await page.title()
      return [before, p.frames().length]
    `),
  )
  expect(counts).toEqual([3, 4])
})

// The one change a script can see: a tab it has not used is read in the background when a reply
// says its frames changed, so a synchronous read right after that operation still lists the tree
// from before. Reading its frames makes the tab used, and the next operation waits for its read.
it('tests that a tab the script never used is read in the background when its frames change', async () => {
  let reads = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: async (call) => {
      if (call.name === 'page.frames') {
        if (tabOf(call) !== 1009 || reads++ === 0) return TREE
        await new Promise((resolve) => setTimeout(resolve, 20))
        return ADDED
      }
      return { ok: 'Agent', frames: [{ tab: 1009, version: 3 }] }
    },
  })
  const counts = json(
    await runtime.run(`
      await page.title()
      const p = ${OTHER}
      const right = p.frames().length
      await page.title()
      return [right, p.frames().length]
    `),
  )
  expect(counts).toEqual([3, 4])
  expect(reads).toEqual(2)
})

// An evaluate's reply holds the page's whole tree as the script left it. The settle read that
// follows (its reply carries the moved version) is a newer observation of the page: one that gets
// no tree must not leave the script the tree from before the evaluate.
it("tests that an evaluate's whole tree stays applied when the settle read after it gets no tree", async () => {
  let reads = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'page.frames') return reads++ === 0 ? TREE : {}
      if (call.name === 'page.evaluate')
        return {
          ok: { value: { n: 1 }, frames: ADDED.frames },
          pages: [{ tab: 1007 }],
          frames: [{ tab: 1007, version: 2 }],
        }
      return null
    },
  })
  const count = json(
    await runtime.run(
      'await page.evaluate(() => 1); return page.frames().length',
    ),
  )
  expect(reads).toEqual(2)
  expect(count).toEqual(4)
})

// The same for a wait outside a tracked navigation: its reply's tree is the last good one.
it("tests that a load-state wait's whole tree stays applied when the settle read after it gets no tree", async () => {
  let reads = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'page.frames') return reads++ === 0 ? TREE : {}
      if (call.name === 'page.waitForLoadState')
        return {
          ok: {
            response: null,
            url: 'https://a.example/',
            frames: ADDED.frames,
            frameSnapshotAttempted: true,
          },
          frames: [{ tab: 1007, version: 2 }],
        }
      return null
    },
  })
  const count = json(
    await runtime.run(
      'await page.waitForLoadState(); return page.frames().length',
    ),
  )
  // The settle step's read, and the one `_navigated` settles with, both got no tree.
  expect(reads).toEqual(3)
  expect(count).toEqual(4)
})

// A wait's read of a tab the script never acted on but whose frames it read is bounded as a
// listed tab's start read is: a tab whose attach stalls, one another run is navigating say, must
// not hold the script's timer.
it('tests that a wait reads a tab the script only read the frames of within the start read bound', async () => {
  let reads1009 = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name !== 'page.frames') return null
      if (tabOf(call) !== 1009 || reads1009++ > 0) return TREE
      return {
        err: {
          code: 'timeout',
          owner: 'host',
          delivery: 'unknown',
          retryable: true,
          operationId: 'run.2',
          timeout: true,
          message: 'page.frames: Timeout 1000ms exceeded',
        },
      }
    },
  })
  await runtime.run(
    `${OTHER}.frames(); await page.title(); await page.waitForTimeout(1)`,
  )
  const waits = runtime.calls
    .filter(
      (call) =>
        call.name === 'page.frames' &&
        Reflect.get(Object(call.input.call), 'api') === 'page.waitForTimeout',
    )
    .map((call) => [
      tabOf(call),
      Reflect.get(Object(call.input.call), 'timeoutMs'),
    ])
  expect(waits).toEqual([
    [1007, 0],
    [1009, 1_000],
  ])
})

it('tests that a rejected operation also leaves the changed tree of a page the script uses read before the script catches it', async () => {
  let reads = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1009 && reads++ > 0 ? ADDED : TREE
      return {
        err: {
          code: 'wait_timeout',
          owner: 'website',
          delivery: 'done',
          retryable: true,
          operationId: 'run.3',
          timeout: true,
          message: 'locator.click: Timeout 5000ms exceeded',
        },
        frames: [{ tab: 1009, version: 3 }],
      }
    },
  })
  const count = json(
    await runtime.run(`
      const p = ${OTHER}
      p.frames()
      try { await page.locator('#go').click() } catch {}
      return p.frames().length
    `),
  )
  expect(count).toEqual(4)
})

it('tests that a page whose frames change between every two operations has its current tree after each', async () => {
  let version = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1009
          ? {
              frames: [
                TREE.frames[0],
                ...Array.from({ length: version }, (_, i) => ({
                  id: `f${i}`,
                  parentId: 'main',
                  name: `f${i}`,
                  url: 'https://c.example/',
                })),
              ],
            }
          : TREE
      version += 1
      return { ok: 'Agent', frames: [{ tab: 1009, version }] }
    },
  })
  const counts = json(
    await runtime.run(`
      const p = ${OTHER}
      p.frames()
      const counts = []
      for (let i = 0; i < 4; i++) {
        await page.title()
        counts.push(p.frames().length)
      }
      return counts
    `),
  )
  // One read of the tab per operation: each sees the frames that operation's reply announced.
  expect(counts).toEqual([2, 3, 4, 5])
  expect(reads(runtime.calls).filter(([tab]) => tab === 1009)).toHaveLength(4)
})

// A host measured a run on a healthy tab at about four seconds while another tab of the browser
// was busy: the start waited for the busy tab's read, then its retry, then every statement waited
// for it again, a second each (the native read's budget). The script never touched that tab. Now
// the start waits for it within its grace for listed tabs, and no statement waits for it.
it('tests that a tab whose frames read never answers holds the run start no longer than its grace, and none of its operations', async () => {
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1009 ? new Promise(() => {}) : TREE
      // Each reply says the busy tab's frames changed, as one still loading would.
      return { ok: 'Agent', frames: [{ tab: 1009, version: 2 }] }
    },
  })
  const outcome = await Promise.race([
    runtime.run(
      'return [await page.title(), await page.title(), page.frames().length]',
    ),
    new Promise((resolve) =>
      setTimeout(() => resolve('held by the tab that never answers'), 2_000),
    ),
  ])
  expect(json(outcome)).toEqual(['Agent', 'Agent', 3])
  // Its start read went out and is still in flight, so no statement asked it again.
  expect(
    reads([...runtime.startCalls, ...runtime.calls]).filter(
      ([tab]) => tab === 1009,
    ),
  ).toEqual([[1009, 'page.frames']])
})

// The observer contract (accept.sh's frames check): a host's run walks another listed tab's
// frames in its first statement. The start waits for that tab's first read within its grace, so a
// tab that answers has its whole tree by then, though nothing in the run touched it.
it('tests that a listed tab that answers within the start grace has its whole tree at the first statement', async () => {
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: async (call) => {
      if (call.name !== 'page.frames') return null
      if (tabOf(call) === 1009)
        await new Promise((resolve) => setTimeout(resolve, 50))
      return TREE
    },
  })
  const names = json(
    await runtime.run(`return ${OTHER}.frames().map((f) => f.name())`),
  )
  expect(names).toEqual(['', 'same', 'cross'])
})

// The grace is one window for every listed tab, read in parallel: three tabs that never answer
// cost the start the grace once, not three times.
it('tests that the start grace is one window for all listed tabs', async () => {
  const runtime = loadPrelude({
    seed: {
      ...SEED,
      pages: [1007, 1009, 1010, 1011].map((tab) => ({
        tab,
        url: `https://${tab}.example/`,
        title: String(tab),
        closed: false,
      })),
    },
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1007 ? TREE : new Promise(() => {})
      return 'A'
    },
  })
  const started = Date.now()
  await runtime.run('return await page.title()')
  const took = Date.now() - started
  expect(took).toBeGreaterThanOrEqual(250)
  expect(took).toBeLessThan(800)
})

it('tests that a tab the script never used whose read got no tree is asked again only once its frames change', async () => {
  let titles = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name === 'page.frames') return tabOf(call) === 1009 ? {} : TREE
      titles += 1
      return titles < 3
        ? 'Agent'
        : { ok: 'Agent', frames: [{ tab: 1009, version: 5 }] }
    },
  })
  await runtime.run(`
    await page.waitForTimeout(150)
    await page.title()
    await page.title()
    await page.title()
    await page.waitForTimeout(1)
  `)
  // The start read, with no retry, and one read once the third reply moved its version: neither
  // the wait nor the first two operations asked it again.
  expect(
    reads([...runtime.startCalls, ...runtime.calls]).filter(
      ([tab]) => tab === 1009,
    ),
  ).toEqual([
    [1009, 'page.frames'],
    [1009, 'page.frames'],
  ])
})

// The script can act on a listed tab while that tab's start read is still in flight. The read then
// answers against an older observation of the tab than the op's reply, so its tree is not applied;
// the tab is marked unknown and read again before the op resolves.
it('tests that a listed tab the script acts on while its start read is in flight has its whole tree when that op resolves', async () => {
  let reads1009 = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: async (call) => {
      if (call.name === 'page.title')
        return { ok: 'Other', pages: [{ tab: 1009 }] }
      if (call.name !== 'page.frames') return null
      if (tabOf(call) !== 1009 || reads1009++ > 0) return TREE
      // Longer than the start's grace for listed tabs, so it is still in flight.
      await new Promise((resolve) => setTimeout(resolve, 400))
      return {
        ok: TREE,
        pages: [{ tab: 1009 }],
        observed: { seq: 1, events: 0, payloadCurrent: true },
      }
    },
  })
  const count = json(
    await runtime.run(`
      const p = ${OTHER}
      await p.title()
      return p.frames().length
    `),
  )
  expect(count).toEqual(3)
  expect(reads1009).toEqual(2)
})

// A page the run first sees after it started holds the script for its reads, so a popup or a new
// page has its whole tree when the call that handed it out resolves. One that does not answer, a
// busy tab another client opened, costs the run that one wait in all: its unanswered read must not
// leave it holding every later statement, which is how the old wait came back for listed tabs.
it('tests that a page first seen mid-run that does not answer costs the run one wait, never one per statement', async () => {
  const timeline: string[] = []
  let titles = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: async (call) => {
      if (call.name === 'page.frames') {
        if (tabOf(call) !== 1011) return TREE
        await new Promise((resolve) => setTimeout(resolve, 40))
        timeline.push('1011 read got no tree')
        return {}
      }
      titles += 1
      timeline.push(`title ${titles}`)
      // The first title's reply names a tab the run had not seen, opened by another client.
      return titles === 1
        ? {
            ok: 'A',
            pages: [{ tab: 1011, url: 'https://c.example/', title: 'C' }],
          }
        : 'A'
    },
  })
  await runtime.run('for (let i = 0; i < 6; i++) await page.title()')
  expect(timeline).toEqual([
    'title 1',
    '1011 read got no tree',
    'title 2',
    'title 3',
    'title 4',
    'title 5',
    'title 6',
  ])
  expect(reads(runtime.calls).filter(([tab]) => tab === 1011)).toEqual([
    [1011, 'page.frames'],
  ])
})

// A tab the script starts using while a background read of it is in flight: that read was sent
// before the op's reply named the tab, so it answers against an older observation and is not
// applied. The op reads the tab once more before it resolves.
it('tests that a tab the script starts using during a background read of it has its current tree when that op resolves', async () => {
  let reads1009 = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: async (call) => {
      if (call.name === 'page.title')
        return tabOf(call) === 1009
          ? { ok: 'Other', pages: [{ tab: 1009 }] }
          : { ok: 'Agent', frames: [{ tab: 1009, version: 3 }] }
      if (call.name !== 'page.frames') return null
      if (tabOf(call) !== 1009) return TREE
      reads1009 += 1
      if (reads1009 === 1) return TREE
      if (reads1009 > 2) return ADDED
      await new Promise((resolve) => setTimeout(resolve, 30))
      return {
        ok: ADDED,
        pages: [{ tab: 1009 }],
        observed: { seq: 3, events: 0, payloadCurrent: true },
      }
    },
  })
  const count = json(
    await runtime.run(`
      await page.title()
      const p = ${OTHER}
      await p.title()
      return p.frames().length
    `),
  )
  expect(count).toEqual(4)
  expect(reads1009).toEqual(3)
})

// A page first seen mid-run spends its one wait for good: one that answers a background read
// between busy spells does not hold the script again.
it('tests that a page first seen mid-run spends its one wait for good, though it later answers', async () => {
  const timeline: string[] = []
  let titles = 0
  let reads1011 = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: async (call) => {
      if (call.name === 'page.frames') {
        if (tabOf(call) !== 1011) return TREE
        reads1011 += 1
        const read = reads1011
        if (read === 2) return TREE
        await new Promise((resolve) => setTimeout(resolve, 40))
        timeline.push(`1011 read ${read} got no tree`)
        return {}
      }
      titles += 1
      timeline.push(`title ${titles}`)
      if (titles === 1)
        return {
          ok: 'A',
          pages: [{ tab: 1011, url: 'https://c.example/', title: 'C' }],
        }
      // Its frames change after its first read, and again after its second.
      if (titles === 2 || titles === 3)
        return { ok: 'A', frames: [{ tab: 1011, version: titles }] }
      return 'A'
    },
  })
  await runtime.run(`
    await page.title()
    await page.title()
    await page.waitForTimeout(20)
    await page.title()
    await page.title()
    await page.waitForTimeout(100)
  `)
  expect(timeline).toEqual([
    'title 1',
    '1011 read 1 got no tree',
    'title 2',
    'title 3',
    'title 4',
    '1011 read 3 got no tree',
  ])
})

it('tests that a page first seen mid-run has its whole tree when the operation that handed it out resolves', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'page.frames') return TREE
      if (call.name === 'context.newPage')
        return { page: { $neo: 'page', tab: 1011 }, url: 'https://a.example/' }
      return 'ok'
    },
  })
  const names = json(
    await runtime.run(`
      const other = await context.newPage()
      return other.frames().map((f) => f.name())
    `),
  )
  expect(names).toEqual(['', 'same', 'cross'])
})

it("tests that the run's init versions keep its first reply from reading unchanged tabs again", async () => {
  const runtime = loadPrelude({
    seed: { ...OBSERVER, frames: [{ tab: 1009, version: 4 }] },
    answer: (call) =>
      call.name === 'page.frames'
        ? TREE
        : { ok: 'Agent', frames: [{ tab: 1009, version: 4 }] },
  })
  await runtime.run('await page.title()')
  expect(reads(runtime.calls).filter(([tab]) => tab === 1009)).toEqual([])
})

it("tests that a navigation whose bounded snapshot got no tree returns with the tab's whole tree", async () => {
  let read = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'page.frames') return read++ === 0 ? MAIN_ONLY : TREE
      if (call.name === 'page.goto')
        return {
          response: null,
          url: 'https://a.example/next',
          frameSnapshotAttempted: true,
        }
      return 'ok'
    },
  })
  const counts = json(
    await runtime.run(`
      const before = page.frames().length
      await page.goto('https://a.example/next')
      return [before, page.frames().length]
    `),
  )
  expect(counts).toEqual([1, 3])
})

it('tests that a page first seen mid-run whose first read was partial has its retried whole tree when its operation resolves', async () => {
  let reads1011 = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1011 && reads1011++ === 0
          ? { ...MAIN_ONLY, incomplete: true }
          : TREE
      if (call.name === 'context.newPage')
        return { page: { $neo: 'page', tab: 1011 }, url: 'https://a.example/' }
      return 'ok'
    },
  })
  const count = json(
    await runtime.run(`
      const other = await context.newPage()
      return other.frames().length
    `),
  )
  expect(count).toEqual(3)
})

// A task's tab B, listed at the start with its whole tree, which the built-in `page` (no tab
// yet) is then bound to by a public task pick.
const B = {
  tab: 1101,
  taskId: 't-b',
  url: 'https://a.example/',
  title: 'B',
  closed: false,
}

it("tests that a public task pick binds the built-in page with its tab's whole tree", async () => {
  const runtime = loadPrelude({
    withNeo: true,
    seed: { page: null, pages: [B], frames: [{ tab: 1101, version: 2 }] },
    answer: (call) => {
      if (call.name === 'page.frames') return TREE
      if (call.name === 'neo.task')
        return {
          ok: { id: 't-b', pages: [{ $neo: 'page', tab: 1101 }] },
          pickedPage: { tab: 1101 },
          pages: [B],
          frames: [{ tab: 1101, version: 2 }],
        }
      return 'ok'
    },
  })
  const seen = json(
    await runtime.run(`
      await neo.task('t-b')
      return [page.tabId, page.frames().length]
    `),
  )
  expect(seen).toEqual([1101, 3])
})

it("tests that a task's new page whose first read was late and partial has its whole tree when newPage resolves", async () => {
  let reads = 0
  const runtime = loadPrelude({
    withNeo: true,
    seed: SEED,
    answer: async (call) => {
      if (call.name === 'page.frames' && tabOf(call) === 1102) {
        // The first read answers late and partial; the retry answers whole.
        if (reads++ === 0) {
          await new Promise((resolve) => setTimeout(resolve, 30))
          return { ...MAIN_ONLY, incomplete: true }
        }
        return TREE
      }
      if (call.name === 'page.frames') return TREE
      if (call.name === 'neo.createTask') return { id: 't-c', pages: [] }
      if (call.name === 'task.newPage')
        return {
          ok: { page: { $neo: 'page', tab: 1102 } },
          pages: [{ tab: 1102, taskId: 't-c', url: 'https://a.example/' }],
          frames: [{ tab: 1102, version: 1 }],
        }
      return 'ok'
    },
  })
  const count = json(
    await runtime.run(`
      const task = await neo.createTask('c')
      const other = await task.newPage()
      return other.frames().length
    `),
  )
  expect(count).toEqual(3)
  expect(reads).toEqual(2)
})

it('tests that a popup whose first read was late and partial has its whole tree once the op that opened it resolves', async () => {
  let reads = 0
  let delivered = false
  const runtime = loadPrelude({
    seed: SEED,
    answer: async (call) => {
      if (call.name === 'page.frames' && tabOf(call) === 1103) {
        if (reads++ === 0) {
          await new Promise((resolve) => setTimeout(resolve, 30))
          return { ...MAIN_ONLY, incomplete: true }
        }
        return TREE
      }
      if (call.name === 'page.frames') return TREE
      if (call.name === 'events.listen') return { since: 0 }
      if (call.name === 'events.next') {
        if (delivered) return new Promise(() => {})
        delivered = true
        return {
          events: [
            {
              seq: 1,
              event: 'popup',
              page: { $neo: 'page', tab: 1007 },
              popup: {
                page: { $neo: 'page', tab: 1103 },
                url: 'https://a.example/',
              },
            },
          ],
          lost: 0,
        }
      }
      return 'ok'
    },
  })
  const seen = json(
    await runtime.run(`
      let popup = null
      page.on('popup', (p) => { popup = p })
      const inListener = []
      page.on('popup', (p) => inListener.push(p.frames().length))
      await page.waitForEvent('popup')
      return [popup.frames().length, inListener]
    `),
  )
  // A listener runs as the event is delivered and cannot wait; the awaited op resolves after
  // the popup's first read, retry included.
  expect(seen).toEqual([3, [1]])
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
