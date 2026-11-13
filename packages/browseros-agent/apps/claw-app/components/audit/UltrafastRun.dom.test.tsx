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
  it('marks the History card with how long ultrafast ran', async () => {
    await render(<TaskRow task={session()} now={Date.now()} />)
    expect(text()).toContain('ultrafast · 15 sec')
  })

  it('shows time, pages and tokens saved beside the status on the task page', async () => {
    await render(<TaskPageHeader session={session()} />)
    expect(text()).toContain('15 sec · 9 pages, 62k tokens saved')
  })

  it('says one page, not one pages', async () => {
    await render(
      <TaskPageHeader
        session={session({
          ultrafast: { durationMs: 8_000, pagesRead: 1, tokensSaved: 4_231 },
        })}
      />,
    )
    expect(text()).toContain('8 sec · 1 page, 4.2k tokens saved')
  })

  it('marks a task neo browsed with pursue the same way, card and page', async () => {
    const browsed = session({
      name: 'Search flights to Singapore',
      ultrafast: { durationMs: 72_400, pagesRead: 5, tokensSaved: 18_204 },
    })
    await render(
      <>
        <TaskRow task={browsed} now={Date.now()} />
        <TaskPageHeader session={browsed} />
      </>,
    )
    expect(text()).toContain('ultrafast · 1 min')
    expect(text()).toContain('1 min · 5 pages, 18k tokens saved')
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
