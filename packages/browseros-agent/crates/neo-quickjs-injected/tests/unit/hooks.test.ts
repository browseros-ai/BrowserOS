/**
 * `createHooks` against the fake `__neo_runtime` (fake-runtime.ts): registration and its JSON,
 * every allowed return and every failure per event, the folding of several handlers, filters,
 * `ctx`, and the real objects a handler drives the browser with. Assertions read decisions,
 * objects and the messages sent; never the wording of an error or a trace line.
 */

import { describe, expect, test } from 'bun:test'
import { Buffer, exportValue } from '../../src/buffer'
import type { HookContext, Hooks, NeoHooks, Ship } from '../../src/hooks'
import { createHooks } from '../../src/index'
import { Locator } from '../../src/locator'
import { Page } from '../../src/page'
import { eventless } from './bridge-stub'
import {
  FakeRuntime,
  OTHER_TAB,
  RUN_TAB,
  start,
  type Tab,
} from './fake-runtime'

/** A hook realm on `RUN_TAB` with `OTHER_TAB` open, the fake installed as `__neo_runtime` and
 * every `log` line kept. `meta` defaults to a sample; passing `undefined` sends none. */
function startHooks(...args: [meta?: unknown]) {
  const meta = args.length ? args[0] : { task: 'sign in' }
  const tabs: Tab[] = [RUN_TAB, OTHER_TAB]
  const fake = new FakeRuntime(tabs)
  for (const tab of tabs) fake.handed.add(`page@${tab.id}`)
  const logs: string[] = []
  const hostRequests: unknown[] = []
  const host = {
    answer: async (_request: unknown): Promise<unknown> => {
      throw new Error('no host')
    },
  }
  const stub = {
    host: async (text: string) => {
      const request: unknown = JSON.parse(text)
      hostRequests.push(request)
      return JSON.stringify(await host.answer(request))
    },
    get: fake.get.bind(fake),
    call: fake.call.bind(fake),
    abort: fake.abort.bind(fake),
    log: (line: string) => {
      logs.push(line)
    },
  }
  Object.assign(globalThis, { __neo_runtime: eventless(stub) })
  const hooks = createHooks({
    page: 'page@RUN',
    mainFrame: 'frame@RUN',
    context: 'context@default',
    browser: 'browser@default',
    url: RUN_TAB.url,
    testIdAttribute: 'data-testid',
    lineOffset: 2,
    meta,
  })
  return { fake, hooks, logs, host, hostRequests }
}

/** Runs a hook file's body and ends loading: the registrations JSON, parsed. */
async function load(
  hooks: Hooks,
  body: (neo: NeoHooks) => unknown,
): Promise<unknown> {
  await body(hooks.neo)
  return JSON.parse(hooks.finishLoading())
}

let nextId = 1

/** One invocation's decision, parsed. */
async function decide(
  hooks: Hooks,
  event: string,
  payload: unknown,
  ship?: Ship,
  marker?: string,
): Promise<Record<string, unknown>> {
  return JSON.parse(
    await hooks.invoke(nextId++, event, JSON.stringify(payload), ship, marker),
  )
}

type Event = Record<string, unknown>

const BEFORE_RUN = {
  script: 'await page.click("#buy")',
  page: 'page@RUN',
  context: 'context@default',
}

function afterRun(status: string, fields: Record<string, unknown> = {}) {
  return { status, page: 'page@RUN', context: 'context@default', ...fields }
}

const FILL = {
  kind: 'fill',
  group: 'type',
  method: 'fill',
  guid: 'frame@RUN',
  args: { selector: '#pw', value: '{{password}}', strict: true },
  location: { line: 3, column: 7 },
  page: 'page@RUN',
  frame: 'frame@RUN',
  context: 'context@default',
  locator: { frame: 'frame@RUN', selector: '#pw' },
}

function action(kind: string | null, group: string | null) {
  return { ...FILL, kind, group }
}

/** A promise that never settles: a handler that waits until something else ends it. */
const forever = () => new Promise<never>(() => {})

/** Lets every queued promise reaction run. */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('neo.on and finishLoading', () => {
  test('registrations come back in order, each filter only where given', async () => {
    const { hooks } = startHooks()
    const registrations = await load(hooks, (neo) => {
      neo.on('beforeRun', () => {})
      neo.on('beforeAction', { kinds: ['fill'], groups: ['type'] }, () => {})
      neo.on('afterAction', { groups: ['navigate'] }, () => {})
      neo.on('afterRun', () => {})
      neo.on('beforeAction', { kinds: [] }, () => {})
    })
    expect(registrations).toEqual([
      { event: 'beforeRun' },
      { event: 'beforeAction', kinds: ['fill'], groups: ['type'] },
      { event: 'afterAction', groups: ['navigate'] },
      { event: 'afterRun' },
      { event: 'beforeAction', kinds: [] },
    ])
  })

  const handler = () => {}
  test.each([
    ['an unknown event', ['beforeClick', handler]],
    ['an event that is not a string', [7, handler]],
    ['no handler', ['beforeRun']],
    ['a handler that is not a function', ['beforeRun', 'block']],
    ['a filter on beforeRun', ['beforeRun', { kinds: ['fill'] }, handler]],
    ['a filter on afterRun', ['afterRun', { groups: ['type'] }, handler]],
    ['an array filter', ['beforeAction', ['fill'], handler]],
    ['a string filter', ['beforeAction', 'fill', handler]],
    [
      'a filter with an unknown key',
      ['beforeAction', { kind: ['fill'] }, handler],
    ],
    [
      'kinds that are not an array',
      ['beforeAction', { kinds: 'fill' }, handler],
    ],
    [
      'groups that hold a non-string',
      ['afterAction', { groups: [1] }, handler],
    ],
    ['an empty filter', ['beforeAction', {}, handler]],
    [
      'a filter of undefined lists',
      ['beforeAction', { kinds: undefined }, handler],
    ],
    ['a filter and two handlers', ['beforeAction', handler, handler]],
  ])('refuses %s with a TypeError and registers nothing', async (_, args) => {
    const { hooks } = startHooks()
    const on = hooks.neo.on as (...args: unknown[]) => void
    expect(() => on(...args)).toThrow(TypeError)
    expect(JSON.parse(hooks.finishLoading())).toEqual([])
  })

  test('a null or undefined filter is no filter, on any event', async () => {
    const { hooks } = startHooks()
    const registrations = await load(hooks, (neo) => {
      neo.on('beforeRun', null, () => {})
      neo.on('afterAction', undefined, () => {})
    })
    expect(registrations).toEqual([
      { event: 'beforeRun' },
      { event: 'afterAction' },
    ])
  })

  test('neo.on throws once loading ended: after finishLoading and from a handler', async () => {
    const { hooks } = startHooks()
    let fromHandler: unknown
    await load(hooks, (neo) => {
      neo.on('beforeRun', () => {
        try {
          neo.on('afterRun', () => {})
        } catch (error) {
          fromHandler = error
        }
      })
    })
    expect(() => hooks.neo.on('afterRun', () => {})).toThrow(TypeError)
    expect(await decide(hooks, 'beforeRun', BEFORE_RUN)).toEqual({
      decision: 'allow',
    })
    expect(fromHandler).toBeInstanceOf(TypeError)
    expect(JSON.parse(hooks.finishLoading())).toEqual([{ event: 'beforeRun' }])
  })

  test('the first invoke ends loading even without finishLoading', async () => {
    const { hooks } = startHooks()
    hooks.neo.on('beforeRun', () => {})
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    expect(() => hooks.neo.on('beforeRun', () => {})).toThrow(TypeError)
  })

  test('a registration keeps its filter when the hook file edits its arrays later', async () => {
    const { hooks } = startHooks()
    const kinds = ['fill']
    let ran = 0
    hooks.neo.on('beforeAction', { kinds }, () => {
      ran++
    })
    kinds.push('click')
    kinds[0] = 'goto'
    expect(JSON.parse(hooks.finishLoading())).toEqual([
      { event: 'beforeAction', kinds: ['fill'] },
    ])
    await decide(hooks, 'beforeAction', action('fill', 'type'))
    await decide(hooks, 'beforeAction', action('click', 'pointer'))
    expect(ran).toBe(1)
  })

  test('every function works detached from its object', async () => {
    const { hooks } = startHooks()
    const { on } = hooks.neo
    on('beforeRun', () => ({ block: 'no' }))
    const { finishLoading, invoke, abort } = hooks
    expect(JSON.parse(finishLoading())).toEqual([{ event: 'beforeRun' }])
    abort(99, 'nothing to abort')
    expect(
      JSON.parse(
        await invoke(nextId++, 'beforeRun', JSON.stringify(BEFORE_RUN)),
      ),
    ).toEqual({ decision: 'block', reason: 'no' })
  })
})

describe('beforeRun', () => {
  test.each([
    ['undefined', () => undefined, { decision: 'allow' }],
    ['{ allow: true }', () => ({ allow: true }), { decision: 'allow' }],
    [
      '{ block }',
      () => ({ block: 'owner approval needed' }),
      { decision: 'block', reason: 'owner approval needed' },
    ],
    [
      '{ block } from an async handler',
      async () => ({ block: 'later' }),
      { decision: 'block', reason: 'later' },
    ],
  ])('%s decides as the contract says', async (_, handler, expected) => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => neo.on('beforeRun', handler))
    expect(await decide(hooks, 'beforeRun', BEFORE_RUN)).toEqual(expected)
  })

  test('no handler allows', async () => {
    const { hooks } = startHooks()
    await load(hooks, () => {})
    expect(await decide(hooks, 'beforeRun', BEFORE_RUN)).toEqual({
      decision: 'allow',
    })
  })

  const BEFORE_RUN_FAILURES: [string, () => unknown][] = [
    [
      'a throw',
      () => {
        throw new Error('policy store down')
      },
    ],
    [
      'a thrown string',
      () => {
        throw 'nope'
      },
    ],
    ['a rejection', () => Promise.reject(new TypeError('x'))],
    ['null', () => null],
    ['a number', () => 42],
    ['a string', () => 'block'],
    ['true', () => true],
    ['an array', () => [{ block: 'x' }]],
    ['an empty object', () => ({})],
    ['{ allow: false }', () => ({ allow: false })],
    ['{ allow: "yes" }', () => ({ allow: 'yes' })],
    ['an empty block', () => ({ block: '' })],
    ['a block that is not a string', () => ({ block: 42 })],
    ['block and allow together', () => ({ block: 'x', allow: true })],
    ['replaceArgs, an action-only decision', () => ({ replaceArgs: {} })],
    ['replace, an after-event decision', () => ({ replace: 1 })],
    ['note, an after-event decision', () => ({ note: 'x' })],
    ['a symbol key', () => ({ [Symbol('allow')]: true })],
    [
      'a hidden extra key',
      () => Object.defineProperty({ allow: true }, 'extra', { value: 1 }),
    ],
    [
      'a getter that throws',
      () => ({
        get block(): string {
          throw new Error('read')
        },
      }),
    ],
  ]

  test.each(BEFORE_RUN_FAILURES)('%s fails', async (_, handler) => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => neo.on('beforeRun', handler))
    const decision = await decide(hooks, 'beforeRun', BEFORE_RUN)
    expect(decision.decision).toBe('fail')
    expect(typeof decision.error).toBe('string')
    expect(Object.keys(decision).sort()).toEqual(['decision', 'error'])
  })

  test('the first block wins and later handlers never run', async () => {
    const { hooks } = startHooks()
    const ran: number[] = []
    await load(hooks, (neo) => {
      neo.on('beforeRun', () => {
        ran.push(1)
      })
      neo.on('beforeRun', () => {
        ran.push(2)
        return { block: 'first' }
      })
      neo.on('beforeRun', () => {
        ran.push(3)
        return { block: 'second' }
      })
    })
    expect(await decide(hooks, 'beforeRun', BEFORE_RUN)).toEqual({
      decision: 'block',
      reason: 'first',
    })
    expect(ran).toEqual([1, 2])
  })

  test('a failure stops later handlers', async () => {
    const { hooks } = startHooks()
    let laterRan = false
    await load(hooks, (neo) => {
      neo.on('beforeRun', () => ({ allow: 'maybe' }))
      neo.on('beforeRun', () => {
        laterRan = true
      })
    })
    expect((await decide(hooks, 'beforeRun', BEFORE_RUN)).decision).toBe('fail')
    expect(laterRan).toBe(false)
  })

  test('event: the script, the run page and its context, and meta', async () => {
    const { hooks, fake } = startHooks({ task: 'sign in' })
    const events: Event[] = []
    await load(hooks, (neo) =>
      neo.on('beforeRun', (event) => {
        events.push(event)
      }),
    )
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    const [event, again] = events as [Event, Event]
    const page = event.page as Page
    expect(event.script).toBe(BEFORE_RUN.script)
    expect(page).toBeInstanceOf(Page)
    expect(again.page).toBe(page)
    expect(page.context()).toBe(event.context as ReturnType<Page['context']>)
    expect(page.context().pages()[0]).toBe(page)
    expect(page.mainFrame()).toBe(page.mainFrame())
    expect(event.meta).toEqual({ task: 'sign in' })
    expect(Object.keys(event).sort()).toEqual([
      'context',
      'meta',
      'page',
      'script',
    ])
    expect(fake.messages).toEqual([])
  })

  test("a hook's page.goto sends one call, on the run page's main frame", async () => {
    const { hooks, fake } = startHooks()
    await load(hooks, (neo) =>
      neo.on('beforeRun', async (event) => {
        await (event.page as Page).goto('https://x.test/')
      }),
    )
    expect(await decide(hooks, 'beforeRun', BEFORE_RUN)).toEqual({
      decision: 'allow',
    })
    expect(
      fake.messages.map(({ guid, method, params }) => [
        guid,
        method,
        (params as { url: string }).url,
      ]),
    ).toEqual([['frame@RUN', 'goto', 'https://x.test/']])
  })
})

describe('beforeAction', () => {
  test.each([
    ['undefined', () => undefined, { decision: 'allow' }],
    ['{ allow: true }', () => ({ allow: true }), { decision: 'allow' }],
    [
      '{ block }',
      () => ({ block: 'not on this site' }),
      { decision: 'block', reason: 'not on this site' },
    ],
    [
      '{ replaceArgs }',
      (event: Event) => ({
        replaceArgs: { ...(event.args as object), value: 'hunter2' },
      }),
      {
        decision: 'replaceArgs',
        args: { selector: '#pw', value: 'hunter2', strict: true },
      },
    ],
  ])('%s decides as the contract says', async (_, handler, expected) => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => neo.on('beforeAction', handler))
    expect(await decide(hooks, 'beforeAction', FILL)).toEqual(expected)
  })

  test('replaceArgs chains: each later handler sees the replaced arguments', async () => {
    const { hooks } = startHooks()
    const seen: unknown[] = []
    await load(hooks, (neo) => {
      neo.on('beforeAction', (event) => {
        seen.push(event.args)
        return { replaceArgs: { ...(event.args as object), value: 'a' } }
      })
      neo.on('beforeAction', (event) => {
        seen.push(event.args)
      })
      neo.on('beforeAction', (event) => {
        seen.push(event.args)
        ;(event.args as Record<string, unknown>).edited = true
        return { replaceArgs: { ...(event.args as object), extra: 1 } }
      })
    })
    expect(await decide(hooks, 'beforeAction', FILL)).toEqual({
      decision: 'replaceArgs',
      args: {
        selector: '#pw',
        value: 'a',
        strict: true,
        edited: true,
        extra: 1,
      },
    })
    expect(seen).toEqual([
      FILL.args,
      { ...FILL.args, value: 'a' },
      { ...FILL.args, value: 'a', edited: true },
    ])
  })

  test("a handler's edits to its event reach no other handler", async () => {
    const { hooks } = startHooks()
    const seen: unknown[] = []
    await load(hooks, (neo) => {
      neo.on('beforeAction', (event) => {
        ;(event.args as Record<string, unknown>).value = 'edited'
        event.kind = 'click'
      })
      neo.on('beforeAction', (event) => {
        seen.push(event.args, event.kind)
      })
    })
    expect(await decide(hooks, 'beforeAction', FILL)).toEqual({
      decision: 'allow',
    })
    expect(seen).toEqual([FILL.args, 'fill'])
  })

  test('a block after a replacement wins', async () => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => {
      neo.on('beforeAction', () => ({ replaceArgs: { value: 'a' } }))
      neo.on('beforeAction', () => ({ block: 'no' }))
    })
    expect(await decide(hooks, 'beforeAction', FILL)).toEqual({
      decision: 'block',
      reason: 'no',
    })
  })

  const cyclic: Record<string, unknown> = {}
  cyclic.self = cyclic
  test.each([
    ['block with replaceArgs', () => ({ block: 'x', replaceArgs: {} })],
    ['replaceArgs null', () => ({ replaceArgs: null })],
    ['replaceArgs an array', () => ({ replaceArgs: ['#pw'] })],
    ['replaceArgs a string', () => ({ replaceArgs: '{}' })],
    [
      'replaceArgs whose JSON is not an object',
      () => ({ replaceArgs: new Date(0) }),
    ],
    ['replaceArgs with a BigInt', () => ({ replaceArgs: { value: 1n } })],
    ['replaceArgs with a cycle', () => ({ replaceArgs: cyclic })],
    ['a replace', () => ({ replace: 'x' })],
    ['an empty block', () => ({ block: '' })],
    ['a rejection', () => Promise.reject(new Error('x'))],
  ])('%s fails', async (_, handler) => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => neo.on('beforeAction', handler))
    expect((await decide(hooks, 'beforeAction', FILL)).decision).toBe('fail')
  })

  test('event: kind, group, method, args, location, and the real page, frame and locator', async () => {
    const { hooks, fake } = startHooks()
    const events: Event[] = []
    await load(hooks, (neo) =>
      neo.on('beforeAction', async (event) => {
        events.push(event)
        await (event.locator as Locator).fill('typed by the hook')
      }),
    )
    expect(await decide(hooks, 'beforeAction', FILL)).toEqual({
      decision: 'allow',
    })
    const [event] = events as [Event]
    const page = event.page as Page
    expect([event.kind, event.group, event.method]).toEqual([
      'fill',
      'type',
      'fill',
    ])
    expect(event.args).toEqual(FILL.args)
    expect(event.location).toEqual(FILL.location)
    expect(page.context().pages()[0]).toBe(page)
    expect(page.context()).toBe(event.context as ReturnType<Page['context']>)
    expect(event.frame).toBe(page.mainFrame())
    expect(event.locator).toBeInstanceOf(Locator)
    expect(event.meta).toEqual({ task: 'sign in' })
    expect('element' in event).toBe(false)
    expect(
      fake.messages.map(({ guid, method, params }) => [
        guid,
        method,
        (params as { selector: string }).selector,
      ]),
    ).toEqual([['frame@RUN', 'fill', '#pw']])
  })

  test('another page is built the way context.pages() builds it, and a frame belongs to its page', async () => {
    const { hooks, fake } = startHooks()
    fake.answer = () => ({})
    const events: Event[] = []
    await load(hooks, (neo) =>
      neo.on('beforeAction', async (event) => {
        events.push(event)
        await (event.frame as ReturnType<Page['mainFrame']>).click('#a')
      }),
    )
    await decide(hooks, 'beforeAction', {
      ...FILL,
      page: 'page@B',
      frame: 'frame@B/F1',
      locator: { frame: 'frame@B', selector: '#b' },
    })
    const [event] = events as [Event]
    const page = event.page as Page
    expect(fake.gets).toContainEqual({ guid: 'page@B', field: 'initializer' })
    expect(page.url()).toBe(OTHER_TAB.url)
    expect(page.context().pages()).toContain(page)
    expect((event.frame as ReturnType<Page['mainFrame']>).page()).toBe(page)
    expect(event.frame).not.toBe(page.mainFrame())
    expect(fake.messages.map(({ guid, method }) => [guid, method])).toEqual([
      ['frame@B/F1', 'click'],
    ])
  })

  test("a payload naming another context gets that context's own object", async () => {
    const { hooks } = startHooks()
    const events: Event[] = []
    await load(hooks, (neo) =>
      neo.on('beforeAction', (event) => {
        events.push(event)
      }),
    )
    await decide(hooks, 'beforeAction', {
      ...FILL,
      context: 'context@X',
      page: 'page@B',
      frame: undefined,
      locator: undefined,
    })
    await decide(hooks, 'beforeAction', action('fill', 'type'))
    const [created, run] = events as [Event, Event]
    expect(created.context).not.toBe(run.context)
    expect((created.page as Page).context()).toBe(
      created.context as ReturnType<Page['context']>,
    )
    expect(
      (created.context as ReturnType<Page['context']>).browser()?.contexts(),
    ).toEqual([run.context, created.context] as ReturnType<Page['context']>[])
  })
})

describe('filters', () => {
  test('a filtered handler runs when the kind or the group matches', async () => {
    const { hooks } = startHooks()
    const ran: string[] = []
    await load(hooks, (neo) => {
      neo.on('beforeAction', { kinds: ['fill'] }, () => {
        ran.push('fill')
      })
      neo.on('beforeAction', { groups: ['pointer'] }, () => {
        ran.push('pointer')
      })
      neo.on('beforeAction', { kinds: ['goto'], groups: ['type'] }, () => {
        ran.push('goto or type')
      })
      neo.on('afterAction', { kinds: ['fill'] }, () => {
        ran.push('after fill')
      })
    })
    await decide(hooks, 'beforeAction', action('fill', 'type'))
    expect(ran).toEqual(['fill', 'goto or type'])
    ran.length = 0
    await decide(hooks, 'beforeAction', action('click', 'pointer'))
    expect(ran).toEqual(['pointer'])
    ran.length = 0
    await decide(hooks, 'beforeAction', action('goto', 'navigate'))
    expect(ran).toEqual(['goto or type'])
    ran.length = 0
    await decide(hooks, 'beforeAction', action('reload', 'navigate'))
    expect(ran).toEqual([])
  })

  test('a null kind or group matches no filter; an unfiltered handler still runs', async () => {
    const { hooks } = startHooks()
    const kinds: unknown[] = []
    let filteredRan = false
    await load(hooks, (neo) => {
      neo.on('beforeAction', { kinds: ['fill'], groups: ['type'] }, () => {
        filteredRan = true
      })
      neo.on('beforeAction', (event) => {
        kinds.push([event.kind, event.group])
      })
    })
    const { kind: _kind, group: _group, ...unclassified } = FILL
    await decide(hooks, 'beforeAction', action(null, null))
    await decide(hooks, 'beforeAction', unclassified)
    expect(filteredRan).toBe(false)
    expect(kinds).toEqual([
      [null, null],
      [null, null],
    ])
  })
})

describe('afterRun', () => {
  test.each([
    ['undefined', () => undefined, { decision: 'keep', notes: [] }],
    [
      '{ note }',
      () => ({ note: 'filtered' }),
      { decision: 'keep', notes: ['filtered'] },
    ],
    [
      '{ replace }',
      () => ({ replace: 'redacted' }),
      { decision: 'replace', value: 'redacted', notes: [] },
    ],
    [
      '{ replace, note }',
      () => ({ replace: { rows: 2 }, note: 'trimmed' }),
      { decision: 'replace', value: { rows: 2 }, notes: ['trimmed'] },
    ],
    [
      '{ replace: null }',
      () => ({ replace: null }),
      { decision: 'replace', value: null, notes: [] },
    ],
    [
      '{ replace: undefined }, which has no value',
      () => ({ replace: undefined }),
      { decision: 'replace', notes: [] },
    ],
  ])('%s decides as the contract says', async (_, handler, expected) => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => neo.on('afterRun', handler))
    const decision = await decide(
      hooks,
      'afterRun',
      afterRun('ok', { value: 1 }),
    )
    expect(decision).toEqual(expected)
    expect(Object.keys(decision).sort()).toEqual(Object.keys(expected).sort())
  })

  test('each handler sees the result as replaced so far, and notes accumulate', async () => {
    const { hooks } = startHooks()
    const seen: unknown[] = []
    await load(hooks, (neo) => {
      neo.on('afterRun', (event) => {
        seen.push(event.value)
        return { replace: `${event.value}a`, note: 'one' }
      })
      neo.on('afterRun', (event) => {
        seen.push(event.value)
        return { note: 'two' }
      })
      neo.on('afterRun', (event) => {
        seen.push(event.value)
        return { replace: `${event.value}!` }
      })
    })
    expect(
      await decide(hooks, 'afterRun', afterRun('ok', { value: 'x' })),
    ).toEqual({
      decision: 'replace',
      value: 'xa!',
      notes: ['one', 'two'],
    })
    expect(seen).toEqual(['x', 'xa', 'xa'])
  })

  test.each(['error', 'timeout', 'cancelled'])(
    'replacing the result of a run that ended %s fails; a note does not',
    async (status) => {
      const { hooks } = startHooks()
      const error = { name: 'Error', message: 'boom' }
      await load(hooks, (neo) => {
        neo.on('afterRun', () => ({ note: 'saw it' }))
        neo.on('afterRun', (event) =>
          event.status === 'ok' ? undefined : { replace: 'hidden' },
        )
      })
      expect(
        (await decide(hooks, 'afterRun', afterRun(status, { error }))).decision,
      ).toBe('fail')
      const { hooks: noting } = startHooks()
      await load(noting, (neo) =>
        neo.on('afterRun', () => ({ note: 'saw it' })),
      )
      expect(
        await decide(noting, 'afterRun', afterRun(status, { error })),
      ).toEqual({
        decision: 'keep',
        notes: ['saw it'],
      })
    },
  )

  const cyclic: Record<string, unknown> = {}
  cyclic.self = cyclic
  test.each([
    ['an empty object', () => ({})],
    ['an unknown key', () => ({ replace: 1, extra: 2 })],
    ['a note that is not a string', () => ({ note: 5 })],
    ['allow, a before-event decision', () => ({ allow: true })],
    ['block, a before-event decision', () => ({ block: 'x' })],
    ['null', () => null],
    [
      'a throw',
      () => {
        throw new Error('x')
      },
    ],
    ['a rejection', () => Promise.reject(new Error('x'))],
    ['a replacement JSON cannot hold (BigInt)', () => ({ replace: 10n })],
    ['a replacement with a cycle', () => ({ replace: cyclic })],
  ])('%s fails', async (_, handler) => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => neo.on('afterRun', handler))
    const decision = await decide(
      hooks,
      'afterRun',
      afterRun('ok', { value: 1 }),
    )
    expect(decision.decision).toBe('fail')
    expect(typeof decision.error).toBe('string')
  })

  test("a replacement's binaries are shipped, marked by index, when neo-runtime passes ship", async () => {
    const { hooks } = startHooks()
    await load(hooks, (neo) =>
      neo.on('afterRun', () => ({
        replace: { shot: Buffer.from([1, 2, 3]), raw: new Uint8Array([9]) },
      })),
    )
    const shipped: number[][] = []
    const ship = (bytes: Uint8Array) => shipped.push([...bytes]) - 1
    expect(
      await decide(hooks, 'afterRun', afterRun('ok'), ship, 'neo:binary:test'),
    ).toEqual({
      decision: 'replace',
      value: {
        shot: { type: 'Buffer', bytes: 3, 'neo:binary:test': 0 },
        raw: { type: 'Uint8Array', bytes: 1, 'neo:binary:test': 1 },
      },
      notes: [],
    })
    expect(shipped).toEqual([[1, 2, 3], [9]])
    expect(hooks.exportValue).toBe(exportValue)
  })

  test("without ship, a replacement is the realm's JSON", async () => {
    const { hooks } = startHooks()
    await load(hooks, (neo) =>
      neo.on('afterRun', () => ({ replace: { shot: Buffer.from([1, 2]) } })),
    )
    expect(await decide(hooks, 'afterRun', afterRun('ok'))).toEqual({
      decision: 'replace',
      value: { shot: { type: 'Buffer', data: [1, 2] } },
      notes: [],
    })
  })

  test('event: status, value or error, the run page and context, and meta', async () => {
    const { hooks } = startHooks()
    const events: Event[] = []
    await load(hooks, (neo) =>
      neo.on('afterRun', (event) => {
        events.push(event)
      }),
    )
    const error = { name: 'TimeoutError', message: 'too slow' }
    await decide(hooks, 'afterRun', afterRun('ok', { value: { a: 1 } }))
    await decide(hooks, 'afterRun', afterRun('ok'))
    await decide(hooks, 'afterRun', afterRun('timeout', { error }))
    const [returned, undefinedValue, timedOut] = events as [Event, Event, Event]
    expect([returned.status, returned.value]).toEqual(['ok', { a: 1 }])
    expect('value' in undefinedValue).toBe(false)
    expect([timedOut.status, timedOut.error, 'value' in timedOut]).toEqual([
      'timeout',
      error,
      false,
    ])
    expect(returned.page).toBeInstanceOf(Page)
    expect((returned.page as Page).context()).toBe(
      returned.context as ReturnType<Page['context']>,
    )
    expect(returned.meta).toEqual({ task: 'sign in' })
  })
})

describe('afterAction', () => {
  test('keep, replace and notes fold as afterRun folds them', async () => {
    const { hooks } = startHooks()
    const seen: unknown[] = []
    await load(hooks, (neo) => {
      neo.on('afterAction', (event) => {
        seen.push(event.result)
        return { note: 'checked' }
      })
      neo.on('afterAction', (event) => {
        seen.push(event.result)
        return { replace: { value: 'masked' }, note: 'masked' }
      })
      neo.on('afterAction', (event) => {
        seen.push(event.result)
      })
    })
    expect(
      await decide(hooks, 'afterAction', {
        ...FILL,
        result: { value: 'secret' },
      }),
    ).toEqual({
      decision: 'replace',
      value: { value: 'masked' },
      notes: ['checked', 'masked'],
    })
    expect(seen).toEqual([
      { value: 'secret' },
      { value: 'secret' },
      { value: 'masked' },
    ])
    const { hooks: keeping } = startHooks()
    await load(keeping, (neo) => neo.on('afterAction', () => undefined))
    expect(await decide(keeping, 'afterAction', FILL)).toEqual({
      decision: 'keep',
      notes: [],
    })
  })

  test("event: beforeAction's fields plus the result or the error", async () => {
    const { hooks } = startHooks()
    const events: Event[] = []
    await load(hooks, (neo) =>
      neo.on('afterAction', (event) => {
        events.push(event)
      }),
    )
    const error = { name: 'TimeoutError', message: 'no box' }
    await decide(hooks, 'afterAction', { ...FILL, result: { ok: true } })
    await decide(hooks, 'afterAction', { ...FILL, error })
    const [succeeded, failed] = events as [Event, Event]
    expect([succeeded.kind, succeeded.method, succeeded.result]).toEqual([
      'fill',
      'fill',
      { ok: true },
    ])
    expect(succeeded.locator).toBeInstanceOf(Locator)
    expect([failed.error, 'result' in failed]).toEqual([error, false])
  })

  test('a block or replaceArgs fails', async () => {
    const { hooks } = startHooks()
    await load(hooks, (neo) => neo.on('afterAction', () => ({ block: 'x' })))
    expect((await decide(hooks, 'afterAction', FILL)).decision).toBe('fail')
    const { hooks: replacing } = startHooks()
    await load(replacing, (neo) =>
      neo.on('afterAction', () => ({ replaceArgs: {} })),
    )
    expect((await decide(replacing, 'afterAction', FILL)).decision).toBe('fail')
  })
})

describe('ctx', () => {
  test('meta is a deep-frozen copy of the run meta', async () => {
    const meta = { task: 'sign in', site: { origins: ['https://bank.test'] } }
    const { hooks } = startHooks(meta)
    let seen: Record<string, unknown> | undefined
    let fromEvent: unknown
    await load(hooks, (neo) =>
      neo.on('beforeRun', (event, ctx) => {
        seen = ctx.meta as Record<string, unknown>
        fromEvent = event.meta
      }),
    )
    meta.site.origins.push('https://evil.test')
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    expect(seen).toEqual({
      task: 'sign in',
      site: { origins: ['https://bank.test'] },
    })
    expect(fromEvent).toBe(seen)
    const site = seen?.site as { origins: string[] }
    expect([
      Object.isFrozen(seen),
      Object.isFrozen(site),
      Object.isFrozen(site.origins),
    ]).toEqual([true, true, true])
  })

  test.each([undefined, null])(
    'meta is null when the host sent %p',
    async (meta) => {
      const { hooks } = startHooks(meta)
      let seen: unknown = 'unset'
      await load(hooks, (neo) =>
        neo.on('beforeRun', (_event, ctx) => {
          seen = ctx.meta
        }),
      )
      await decide(hooks, 'beforeRun', BEFORE_RUN)
      expect(seen).toBeNull()
    },
  )

  test('state is one object for every handler of every event', async () => {
    const { hooks } = startHooks()
    const states: unknown[] = []
    await load(hooks, (neo) => {
      neo.on('beforeRun', (_event, ctx) => {
        states.push(ctx.state)
        ctx.state.attempts = 1
      })
      neo.on('beforeAction', (_event, ctx) => {
        states.push(ctx.state)
        ctx.state.attempts = (ctx.state.attempts as number) + 1
      })
      neo.on('afterRun', (_event, ctx) => {
        states.push(ctx.state)
        return { note: `attempts ${ctx.state.attempts}` }
      })
    })
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    await decide(hooks, 'beforeAction', FILL)
    await decide(hooks, 'beforeAction', FILL)
    expect(await decide(hooks, 'afterRun', afterRun('ok'))).toEqual({
      decision: 'keep',
      notes: ['attempts 3'],
    })
    expect(new Set(states).size).toBe(1)
    expect(states).toHaveLength(4)
  })

  test("abort(id) aborts only that invocation's signal, and no later handler starts", async () => {
    const { hooks } = startHooks()
    const signals: AbortSignal[] = []
    let laterRan = false
    await load(hooks, (neo) => {
      neo.on(
        'beforeRun',
        (_event, ctx) =>
          new Promise((resolve) => {
            signals.push(ctx.signal)
            // Ignores the abort and allows anyway, once it fired.
            ctx.signal.addEventListener('abort', () => resolve(undefined))
          }),
      )
      neo.on('beforeRun', () => {
        laterRan = true
      })
    })
    const first = hooks.invoke(101, 'beforeRun', JSON.stringify(BEFORE_RUN))
    const second = hooks.invoke(102, 'beforeRun', JSON.stringify(BEFORE_RUN))
    const [mine, other] = signals as [AbortSignal, AbortSignal]
    hooks.abort(999, 'unknown id')
    expect([mine.aborted, other.aborted]).toEqual([false, false])
    hooks.abort(101, 'budget')
    expect([mine.aborted, other.aborted]).toEqual([true, false])
    expect(mine.reason).toBeInstanceOf(Error)
    expect(JSON.parse(await first).decision).toBe('fail')
    expect(laterRan).toBe(false)
    hooks.abort(102, 'cancelled')
    expect(JSON.parse(await second).decision).toBe('fail')
    hooks.abort(101, 'after the end')
  })

  test("a hook's own browser call stops when its signal aborts", async () => {
    const { hooks, fake } = startHooks()
    fake.hold = true
    await load(hooks, (neo) =>
      neo.on('beforeRun', async (event, ctx) => {
        // Neo's option types do not declare Playwright's `signal`; its calls take it anyway.
        await (event.page as Page).goto('https://x.test/', {
          signal: ctx.signal,
        } as never)
      }),
    )
    const pending = hooks.invoke(201, 'beforeRun', JSON.stringify(BEFORE_RUN))
    await flush()
    expect(fake.messages.map(({ method }) => method)).toEqual(['goto'])
    hooks.abort(201, 'budget')
    expect(JSON.parse(await pending).decision).toBe('fail')
    expect(fake.aborts.map(({ id }) => id)).toEqual([fake.messages[0]?.id])
  })

  test('host rejects with an Error', async () => {
    const { hooks } = startHooks()
    let rejected: unknown
    await load(hooks, (neo) =>
      neo.on('beforeRun', async (_event, ctx) => {
        try {
          await ctx.host('vault.login', { origin: 'https://bank.test' })
        } catch (error) {
          rejected = error
        }
      }),
    )
    expect(await decide(hooks, 'beforeRun', BEFORE_RUN)).toEqual({
      decision: 'allow',
    })
    expect(rejected).toBeInstanceOf(Error)
  })

  test('host exchanges JSON and defaults args to null', async () => {
    const { hooks, host, hostRequests } = startHooks()
    host.answer = async (request) => ({ request, items: [true, null, '答'] })
    const answers: unknown[] = []
    await load(hooks, (neo) =>
      neo.on('beforeRun', async (_event, ctx) => {
        answers.push(
          await ctx.host('vault.login', { origin: 'https://bank.test' }),
        )
        answers.push(await ctx.host('empty'))
      }),
    )
    expect((await decide(hooks, 'beforeRun', BEFORE_RUN)).decision).toBe(
      'allow',
    )
    expect(hostRequests).toEqual([
      { name: 'vault.login', args: { origin: 'https://bank.test' } },
      { name: 'empty', args: null },
    ])
    expect(answers).toEqual(
      hostRequests.map((request) => ({ request, items: [true, null, '答'] })),
    )
  })

  test('host rejects invalid names and non-JSON args before sending them', async () => {
    const { hooks, hostRequests } = startHooks()
    const cyclic: Record<string, unknown> = {}
    cyclic.self = cyclic
    const errors: unknown[] = []
    await load(hooks, (neo) =>
      neo.on('beforeRun', async (_event, ctx) => {
        for (const [name, args] of [
          [null, null],
          ['', null],
          [7, null],
          ['x', 1n],
          ['x', cyclic],
          ['x', () => {}],
          ['x', Symbol('x')],
        ]) {
          try {
            await ctx.host(name as string, args)
          } catch (error) {
            errors.push(error)
          }
        }
      }),
    )
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    expect(errors).toHaveLength(7)
    for (const error of errors) expect(error).toBeInstanceOf(TypeError)
    expect(hostRequests).toEqual([])
  })

  test('host turns a native rejection into an Error', async () => {
    const { hooks, host } = startHooks()
    host.answer = async () => {
      throw 'host refused'
    }
    let rejected: unknown
    await load(hooks, (neo) =>
      neo.on('beforeRun', async (_event, ctx) => {
        try {
          await ctx.host('x')
        } catch (error) {
          rejected = error
        }
      }),
    )
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    expect(rejected).toBeInstanceOf(Error)
  })

  test('a saved context cannot ask the host after its invocation ends', async () => {
    const { hooks, host, hostRequests } = startHooks()
    host.answer = async () => null
    let saved: HookContext | undefined
    await load(hooks, (neo) =>
      neo.on('beforeRun', (_event, ctx) => {
        saved = ctx
      }),
    )
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    if (!saved) throw new Error('the hook did not run')
    await expect(saved.host('late')).rejects.toBeInstanceOf(Error)
    expect(hostRequests).toEqual([])
  })

  test('ctx is frozen, one per invocation and shared by its handlers', async () => {
    const { hooks } = startHooks()
    const contexts: unknown[] = []
    await load(hooks, (neo) => {
      neo.on('beforeRun', (_event, ctx) => {
        contexts.push(ctx)
      })
      neo.on('beforeRun', (_event, ctx) => {
        contexts.push(ctx)
      })
    })
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    await decide(hooks, 'beforeRun', BEFORE_RUN)
    const [a, b, c] = contexts as [object, object, object]
    expect(Object.isFrozen(a)).toBe(true)
    expect(b).toBe(a)
    expect(c).not.toBe(a)
  })
})

describe('trace', () => {
  test('each call is one line of the hook trace, every value printed', async () => {
    const { hooks, logs } = startHooks()
    const cyclic: Record<string, unknown> = {}
    cyclic.self = cyclic
    const unprintable = {
      toJSON() {
        throw new Error('json')
      },
      toString() {
        throw new Error('string')
      },
    }
    await load(hooks, (neo) =>
      neo.on('beforeRun', (_event, ctx) => {
        ctx.trace('checked', 2, { ok: true })
        ctx.trace(cyclic, 10n, unprintable, new TypeError('e'))
      }),
    )
    expect(await decide(hooks, 'beforeRun', BEFORE_RUN)).toEqual({
      decision: 'allow',
    })
    expect(logs).toHaveLength(2)
    for (const part of ['checked', '2', '{"ok":true}'])
      expect(logs[0]).toContain(part)
    for (const part of ['10', 'TypeError']) expect(logs[1]).toContain(part)
  })
})

describe('invoke never rejects', () => {
  test.each([
    ['a payload that is not JSON', 'beforeRun', '{script:'],
    ['a payload that is an array', 'beforeRun', '[]'],
    ['a payload that is a string', 'afterRun', '"ok"'],
    ['an unknown event', 'beforeClick', JSON.stringify(BEFORE_RUN)],
    [
      'a payload without its page',
      'beforeRun',
      JSON.stringify({ script: '', context: 'context@default' }),
    ],
    [
      'a guid Rust does not know',
      'beforeRun',
      JSON.stringify({ ...BEFORE_RUN, page: 'page@GONE' }),
    ],
    [
      'a kind that is not a string',
      'beforeAction',
      JSON.stringify({ ...FILL, kind: 3 }),
    ],
    [
      'a locator without a selector',
      'beforeAction',
      JSON.stringify({ ...FILL, locator: { frame: 'frame@RUN' } }),
    ],
  ])('%s fails', async (_, event, payload) => {
    const { hooks } = startHooks()
    let ran = false
    await load(hooks, (neo) => {
      for (const name of ['beforeRun', 'afterRun', 'beforeAction'] as const)
        neo.on(name, () => {
          ran = true
        })
    })
    const decision = JSON.parse(await hooks.invoke(nextId++, event, payload))
    expect(decision.decision).toBe('fail')
    expect(typeof decision.error).toBe('string')
    expect(ran).toBe(false)
  })

  test('an unhandled listener error fails the invocation running and every later one', async () => {
    const { hooks } = startHooks()
    let laterRan = false
    await load(hooks, (neo) => {
      neo.on('beforeRun', (event) => {
        const page = event.page as Page
        page.on('custom', async () => {
          throw new Error('listener failed')
        })
        page.emit('custom')
        return forever()
      })
      neo.on('afterRun', () => {
        laterRan = true
      })
    })
    expect((await decide(hooks, 'beforeRun', BEFORE_RUN)).decision).toBe('fail')
    await expect(hooks.uncaught).rejects.toBeInstanceOf(Error)
    expect((await decide(hooks, 'afterRun', afterRun('ok'))).decision).toBe(
      'fail',
    )
    expect(laterRan).toBe(false)
  })
})

describe('createHooks', () => {
  test("gives createRun's exportValue and standard globals", () => {
    const { hooks } = startHooks()
    const { exportValue: runExport, globals: runGlobals } = start()
    expect(hooks.exportValue).toBe(runExport)
    expect(Object.keys(hooks.globals).sort()).toEqual(
      Object.keys(runGlobals).sort(),
    )
  })

  test('reads and sends nothing until a handler does', async () => {
    const { hooks, fake, logs } = startHooks()
    await load(hooks, (neo) => {
      neo.on('beforeRun', () => {})
      neo.on('afterRun', () => {})
    })
    expect([fake.gets, fake.messages, logs]).toEqual([[], [], []])
  })
})
