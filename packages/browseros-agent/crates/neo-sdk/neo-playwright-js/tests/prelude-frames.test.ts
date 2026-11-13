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
  // The tree it could not read is read again once the script's next operation is answered.
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

it('tests that a listed tab whose start read failed is read again at the next wait', async () => {
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
  const count = json(
    await runtime.run(`
      await page.waitForTimeout(1)
      return ${OTHER}.frames().length
    `),
  )
  expect(count).toEqual(3)
  expect(reads(runtime.calls)).toEqual([
    [1007, 'page.waitForTimeout'],
    [1009, 'page.waitForTimeout'],
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

it("tests that the run's own page whose start read came back short has its whole tree after the script's first awaited operation", async () => {
  for (const short of [{ ...MAIN_ONLY, incomplete: true }, {}, REFUSED_READ]) {
    let read = 0
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => {
        if (call.name === 'page.frames') return read++ === 0 ? short : TREE
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
  }
})

it("tests that a frame added to another task's tab mid-run shows after the script's next awaited operation, with no wait", async () => {
  let reads = 0
  const added = {
    frames: [
      ...TREE.frames,
      { id: 'late', parentId: 'main', name: 'late', url: 'https://c.example/' },
    ],
  }
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1009 && reads++ > 0 ? added : TREE
      // The reply of the script's op says tab 1009's frames changed.
      return {
        ok: 'Agent',
        pages: [
          {
            tab: 1009,
            url: 'https://a.example/',
            closed: false,
            framesChanged: 3,
          },
        ],
        observed: { seq: 1, events: 0 },
      }
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

it('tests that a page whose frames change on every operation is read at most once per 250 ms', async () => {
  let version = 0
  const runtime = loadPrelude({
    seed: OBSERVER,
    now: 1_000,
    answer: (call) => {
      if (call.name === 'page.frames') return TREE
      version += 1
      return {
        ok: 'Agent',
        pages: [
          {
            tab: 1009,
            url: 'https://a.example/',
            closed: false,
            framesChanged: version,
          },
        ],
        observed: { seq: version, events: 0 },
      }
    },
  })
  await runtime.run('for (let i = 0; i < 10; i++) await page.title()')
  expect(reads(runtime.calls).filter(([tab]) => tab === 1009)).toEqual([
    [1009, 'page.frames'],
  ])
})

it("tests that a page first seen mid-run has its whole tree by the script's next await", async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'page.frames') return TREE
      if (call.name === 'test.page')
        return {
          ok: { $neo: 'page', tab: 1011 },
          pages: [{ tab: 1011, url: 'https://a.example/', closed: false }],
          observed: { seq: 1, events: 0 },
        }
      return 'ok'
    },
  })
  const names = json(
    await runtime.run(`
      const other = await __neo.invoke('test.page', {})
      await page.title()
      return other.frames().map((f) => f.name())
    `),
  )
  expect(names).toEqual(['', 'same', 'cross'])
})

// A reply carries the facts of the tabs its op touched, so an op on the script's own page says
// nothing of another task's tab. Every reply carries the browser's frame-change head instead.
it("tests that a frame added to a tab the script reads shows after an operation on another page, from the reply's head", async () => {
  let reads = 0
  const added = {
    frames: [
      ...TREE.frames,
      { id: 'late', parentId: 'main', name: 'late', url: 'https://c.example/' },
    ],
  }
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) => {
      if (call.name === 'page.frames')
        return tabOf(call) === 1009 && reads++ > 0 ? added : TREE
      return { ok: 'Agent', framesHead: 5 }
    },
  })
  const outcome = json(
    await runtime.run(`
      const p = ${OTHER}
      const before = p.frames().length
      await page.title()
      return [before, p.frames().length]
    `),
  )
  expect(outcome).toEqual([3, 4])
})

it('tests that a moved head reads no page the script neither acted on nor read the frames of', async () => {
  const runtime = loadPrelude({
    seed: OBSERVER,
    answer: (call) =>
      call.name === 'page.frames' ? TREE : { ok: 'Agent', framesHead: 9 },
  })
  await runtime.run('await page.title()')
  expect(reads(runtime.calls).filter(([tab]) => tab === 1009)).toEqual([])
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
