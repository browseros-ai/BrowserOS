/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { SessionSummary } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'

const globalNames = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'ResizeObserver',
  'getComputedStyle',
] as const
const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

const client = await import('@/modules/api/client')

// The screenshot base comes from a browser pref this test has no browser for.
mock.module('@/modules/api/client', () => ({
  ...client,
  resolveApiBaseUrl: async () => 'http://127.0.0.1:9200',
}))

const { TaskRow } = await import('./TaskRow')
const { TaskPageHeader } = await import('@/screens/task-detail/TaskPageHeader')
const { NeoSteps } = await import('@/screens/task-detail/NeoSteps')

/** A finished task where a skill ran without the agent, shaped like a real run. */
function session(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: 'f392a113',
    slug: 'claude',
    label: 'claude',
    name: 'Check phone deals on Flipkart',
    startedAt: Date.now() - 60_000,
    durationMs: 53_011,
    dispatchCount: 21,
    toolSequence: ['run_skill', 'run'],
    status: 'done',
    errorCount: 0,
    ultrafast: { durationMs: 15_354, pagesRead: 9, tokensSaved: 61_942 },
    ...overrides,
  }
}

let root: Root
let body: HTMLElement

function define(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  Object.defineProperty(dom.window, 'matchMedia', {
    configurable: true,
    value: () => ({ matches: true }),
  })
  define('window', dom.window)
  define('document', dom.document)
  define('navigator', dom.window.navigator)
  define('HTMLElement', dom.window.HTMLElement)
  define('Node', dom.window.Node)
  define('Event', dom.window.Event)
  define('requestAnimationFrame', (cb: FrameRequestCallback) =>
    setTimeout(() => cb(performance.now()), 16),
  )
  define('cancelAnimationFrame', (id: number) => clearTimeout(id))
  define(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  )
  define('getComputedStyle', () => ({ direction: 'ltr' }))
  define('IS_REACT_ACT_ENVIRONMENT', true)
  body = dom.document.body as unknown as HTMLElement
  const container = dom.document.getElementById(
    'root',
  ) as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

async function render(node: ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter>{node}</MemoryRouter>
      </QueryClientProvider>,
    ),
  )
}

function text(): string {
  return (body.textContent ?? '').replace(/\s+/g, ' ')
}

describe('ultrafast run', () => {
  it('writes what ultrafast did under the History card', async () => {
    await render(<TaskRow task={session()} now={Date.now()} />)
    expect(text()).toContain('ultrafastneo ran it in 15 sec, read 9 pages')
  })

  it('adds the tokens saved on the task page', async () => {
    await render(<TaskPageHeader session={session()} />)
    expect(text()).toContain(
      'ultrafastneo ran it in 15 sec, read 9 pages, 62k tokens saved',
    )
  })

  it('says one page, not one pages', async () => {
    await render(
      <TaskPageHeader
        session={session({
          ultrafast: { durationMs: 8_000, pagesRead: 1, tokensSaved: 4_231 },
        })}
      />,
    )
    expect(text()).toContain('read 1 page, 4.2k tokens saved')
  })

  it('says nothing about ultrafast on a task that ran the normal way', async () => {
    await render(
      <>
        <TaskRow task={session({ ultrafast: undefined })} now={Date.now()} />
        <TaskPageHeader session={session({ ultrafast: undefined })} />
      </>,
    )
    expect(text()).not.toContain('ultrafast')
    expect(text()).not.toContain('tokens saved')
  })
})

/** The recorded Google Flights task: pursue handed back after two clicks and typing. */
const flights = {
  outcome: 'handed_back',
  durationMs: 5_662,
  steps: [
    { action: 'click' },
    { action: 'click' },
    { action: 'type', text: 'Singapore' },
  ],
} as const satisfies NonNullable<SessionSummary['pursue']>

/** The recorded amazon.in task: pursue finished after typing and pressing Enter. */
const colgate = {
  outcome: 'completed',
  durationMs: 7_317,
  steps: [
    { action: 'type', text: 'colgate toothpaste' },
    { action: 'press', text: 'Enter' },
  ],
} as const satisfies NonNullable<SessionSummary['pursue']>

describe('a task neo drove with pursue', () => {
  it('says how many steps neo did and that it handed back', async () => {
    const task = session({ ultrafast: undefined, pursue: flights })
    await render(
      <>
        <TaskRow task={task} now={Date.now()} />
        <TaskPageHeader session={task} />
      </>,
    )
    expect(text()).toContain('neo did 3 steps in 6 sec, then handed back')
    expect(text()).not.toContain('tokens saved')
  })

  it('says how long neo took when it finished', async () => {
    const task = session({ ultrafast: undefined, pursue: colgate })
    await render(
      <>
        <TaskRow task={task} now={Date.now()} />
        <TaskPageHeader session={task} />
      </>,
    )
    expect(text()).toContain('neo finished it in 7 sec')
    expect(text()).not.toContain('steps')
  })

  it('adds pages and tokens on the task page once neo counted them', async () => {
    const task = session({
      pursue: flights,
      ultrafast: { durationMs: 5_662, pagesRead: 4, tokensSaved: 12_310 },
    })
    await render(<TaskPageHeader session={task} />)
    expect(text()).toContain(
      'neo did 3 steps in 6 sec, then handed back, read 4 pages, 12k tokens saved',
    )
  })

  it('lists the steps neo took and where it handed back', async () => {
    await render(<NeoSteps pursue={flights} />)
    const heading = body.querySelector('h2')
    expect(heading?.textContent).toBe('What neo did for your agent')
    const steps = [...body.querySelectorAll('li')].map((item) =>
      (item.textContent ?? '').replace(/^\d+/, ''),
    )
    expect(steps).toEqual(['Clicked', 'Clicked', 'Typed "Singapore"'])
    expect(text()).toContain('Then neo handed the page back to your agent.')
  })

  it('lists the steps of a finished run without a hand-back line', async () => {
    await render(<NeoSteps pursue={colgate} />)
    const steps = [...body.querySelectorAll('li')].map((item) =>
      (item.textContent ?? '').replace(/^\d+/, ''),
    )
    expect(steps).toEqual(['Typed "colgate toothpaste"', 'Pressed Enter'])
    expect(text()).toContain('neo reached the goal on its own.')
    expect(text()).not.toContain('handed')
  })
})

describe('the margin rule on ultrafast tasks', () => {
  const rule = () => body.querySelectorAll('[data-ultrafast-rail]').length
  const rows = () =>
    [...body.querySelectorAll('[data-task-row]')].map((row) =>
      row.hasAttribute('data-ultrafast'),
    )

  it('rules a skill run card and writes its line, with no badge or pill', async () => {
    await render(<TaskRow task={session()} now={Date.now()} />)
    expect(rule()).toBe(1)
    expect(text()).toContain('ultrafastneo ran it in 15 sec')
    expect(text()).not.toContain('ultrafast ·')
  })

  it('rules a pursue card the same way', async () => {
    await render(
      <TaskRow
        task={session({ ultrafast: undefined, pursue: flights })}
        now={Date.now()}
      />,
    )
    expect(rule()).toBe(1)
    expect(text()).toContain('neo did 3 steps')
  })

  it('leaves a task that ran the normal way as it was', async () => {
    await render(
      <TaskRow task={session({ ultrafast: undefined })} now={Date.now()} />,
    )
    expect(rule()).toBe(0)
    expect(rows()).toEqual([false])
  })

  it('flags each ultrafast row so a run of them reads as one rule', async () => {
    await render(
      <div>
        <TaskRow task={session({ sessionId: 'a' })} now={Date.now()} />
        <TaskRow
          task={session({ sessionId: 'b', pursue: colgate })}
          now={Date.now()}
        />
        <TaskRow
          task={session({ sessionId: 'c', ultrafast: undefined })}
          now={Date.now()}
        />
      </div>,
    )
    expect(rows()).toEqual([true, true, false])
    expect(rule()).toBe(2)
  })

  it('writes the line on an ultrafast task page only', async () => {
    await render(<TaskPageHeader session={session()} />)
    expect(text()).toContain('ultrafastneo ran it in 15 sec')
    await render(<TaskPageHeader session={session({ ultrafast: undefined })} />)
    expect(text()).not.toContain('ultrafast')
  })

  it('rules the list of steps neo took and numbers them plainly', async () => {
    await render(<NeoSteps pursue={flights} />)
    expect(rule()).toBe(1)
    expect(
      [...body.querySelectorAll('li')].map(
        (item) => item.firstElementChild?.textContent,
      ),
    ).toEqual(['1', '2', '3'])
  })
})
