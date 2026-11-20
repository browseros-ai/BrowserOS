/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  afterAll,
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  mock,
} from 'bun:test'
import type { Skill } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'
import type { ToastInput } from '@/components/motion/animated-toast-stack'

const api = await import('@/modules/api/client')

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getSession: async () => ({
      session: { durationMs: 5 * 60_000 },
      dispatches: [],
    }),
  }),
}))

const { NowUltrafastStrip, useUpgradeToasts } = await import('./NowUltrafast')
const { connectToastHost } = await import('@/modules/toast/toast')

afterAll(() => mock.restore())

const globalNames = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'localStorage',
] as const
const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

let root: Root
let container: HTMLElement

function define(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

function memoryStorage() {
  const values = new Map<string, string>()
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
    removeItem: (key: string) => void values.delete(key),
  }
}

let created = 1

function taskSkill(patch: Partial<Skill> = {}): Skill {
  created += 1
  return {
    name: 'neo-flipkart-phones',
    description: 'Find phones under a budget',
    origin: 'history',
    sourceSessionId: 'session-1',
    version: 1,
    linkedAgents: [],
    runCount: 1,
    cleanRunCount: 1,
    createdAt: created,
    updatedAt: created,
    ultrafast: {
      status: 'verified',
      flowVersion: 1,
      writeApproved: false,
      p50Ms: 7300,
      verifiedAt: 10,
    },
    ...patch,
  }
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  define('window', dom.window)
  define('document', dom.document)
  define('navigator', dom.window.navigator)
  define('HTMLElement', dom.window.HTMLElement)
  define('Node', dom.window.Node)
  define('Event', dom.window.Event)
  define('localStorage', memoryStorage())
  define('IS_REACT_ACT_ENVIRONMENT', true)
  container = dom.document.getElementById('root') as unknown as HTMLElement
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
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter>{node}</MemoryRouter>
      </QueryClientProvider>,
    )
  })
  for (let tick = 0; tick < 5; tick += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
}

async function remount(node: ReactNode) {
  await act(async () => root.unmount())
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
  await render(node)
}

function text(): string {
  return (container.textContent ?? '').replace(/\s+/g, ' ')
}

function button(label: string): HTMLElement | undefined {
  return [...container.querySelectorAll('button')].find(
    (candidate) => (candidate.textContent ?? '').trim() === label,
  ) as HTMLElement | undefined
}

describe('the now ultrafast strip', () => {
  it('tests that it names the speed against the task once, then stays away', async () => {
    const skill = taskSkill()
    await render(<NowUltrafastStrip skill={skill} />)

    expect(container.querySelector('[role="status"]')).not.toBeNull()
    expect(text()).toContain('Your skill is now ultrafast.')
    expect(text()).toContain(
      'neo learned it on the first run. Next time neo does it by itself.',
    )
    expect(text()).toContain('7s a run, down from about 5 min')

    await remount(<NowUltrafastStrip skill={skill} />)
    expect(text()).not.toContain('Your skill is now ultrafast.')
  })

  it('tests that a skill that still runs with the agent makes no promise to run by itself', async () => {
    const verified = {
      status: 'verified' as const,
      flowVersion: 1,
      writeApproved: false,
      p50Ms: 7300,
      verifiedAt: 10,
    }
    await render(
      <NowUltrafastStrip
        skill={taskSkill({
          ultrafast: {
            ...verified,
            effects: 'write',
            runs: {
              mode: 'with_agent',
              chosen: false,
              reask: false,
              alwaysWithAgent: 'order',
            },
          },
        })}
      />,
    )
    expect(text()).toContain('Your skill is now ultrafast.')
    expect(text()).toContain('neo learned the fast version on the first run.')
    expect(text()).not.toContain('Next time neo does it by itself.')

    await remount(
      <NowUltrafastStrip
        skill={taskSkill({
          ultrafast: {
            ...verified,
            effects: 'write',
            runs: { mode: 'without_agent', chosen: true, reask: false },
          },
        })}
      />,
    )
    expect(text()).toContain(
      'neo learned it on the first run. Next time neo does it by itself.',
    )
  })

  it('tests that Got it puts it away', async () => {
    await render(<NowUltrafastStrip skill={taskSkill()} />)

    await act(async () => button('Got it')?.click())
    expect(text()).not.toContain('Your skill is now ultrafast.')
  })

  it('tests that a skill not made from a task, or not verified, has no strip', async () => {
    await render(<NowUltrafastStrip skill={taskSkill({ origin: 'agent' })} />)
    expect(text()).not.toContain('now ultrafast')

    await remount(
      <NowUltrafastStrip
        skill={taskSkill({
          ultrafast: {
            status: 'pending',
            flowVersion: 0,
            writeApproved: false,
          },
        })}
      />,
    )
    expect(text()).not.toContain('now ultrafast')
  })
})

describe('the now ultrafast toast on Skills', () => {
  function Toasts({ skills }: { skills: Skill[] }) {
    useUpgradeToasts(skills)
    return null
  }

  it('tests that each skill that turned ultrafast gets one toast, once', async () => {
    const shown: ToastInput[] = []
    const disconnect = connectToastHost({
      show: (input) => {
        shown.push(input)
        return String(shown.length)
      },
      update: () => {},
    })
    const upgraded = taskSkill()
    const waiting = taskSkill({
      name: 'neo-other',
      ultrafast: { status: 'none', flowVersion: 0, writeApproved: false },
    })

    await render(<Toasts skills={[upgraded, waiting]} />)
    await remount(<Toasts skills={[upgraded, waiting]} />)
    disconnect()

    // Toasts raised before the host connected (by other test files) flush first; keep ours.
    const upgrades = shown.filter((input) => input.action?.label === 'Open')
    expect(upgrades).toHaveLength(1)
  })
})
