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
import type { SkillDetail as Detail, SkillUltrafast } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes } from 'react-router'

const api = await import('@/modules/api/client')

let ultrafast: SkillUltrafast

const detail = (): Detail => ({
  skill: {
    name: 'reorder-toothpaste',
    description: 'Reorder my usual toothpaste',
    site: 'shop.example.com',
    origin: 'agent',
    version: 1,
    linkedAgents: [],
    runCount: 3,
    cleanRunCount: 3,
    createdAt: 0,
    updatedAt: 0,
    ultrafast,
  },
  body: '',
  runs: [],
  tokenSavings: { saved: 0, otherBrowsers: 0, used: 0, measuredRunCount: 0 },
})

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getSkill: async () => detail(),
  }),
}))

// These pull in their own server calls and animated menus, which this page
// test does not exercise.
mock.module('@/components/skills/RunSkillButton', () => ({
  RunSkillButton: () => null,
}))
mock.module('@/components/skills/SkillActions', () => ({
  SkillActions: () => null,
}))
mock.module('@/components/skills/RunsChoiceDialog', () => ({
  RunsChoiceDialog: () => null,
}))
mock.module('@/components/motion/collapsible', () => ({
  Collapsible: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  CollapsibleTrigger: ({ children }: { children: ReactNode }) => (
    <button type="button">{children}</button>
  ),
  CollapsibleIndicator: () => null,
  CollapsibleContent: () => null,
}))

const { SkillDetail } = await import('./SkillDetail')

afterAll(() => mock.restore())

const globalNames = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
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

beforeEach(async () => {
  ultrafast = {
    status: 'verified',
    flowVersion: 0,
    effects: 'write',
    writeApproved: false,
    p50Ms: 1200,
  }
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  define('window', dom.window)
  define('document', dom.document)
  define('navigator', dom.window.navigator)
  define('HTMLElement', dom.window.HTMLElement)
  define('Node', dom.window.Node)
  define('Event', dom.window.Event)
  define('requestAnimationFrame', (run: FrameRequestCallback) =>
    setTimeout(() => run(0), 0),
  )
  define('cancelAnimationFrame', (id: number) => clearTimeout(id))
  define('getComputedStyle', () => ({ getPropertyValue: () => '' }))
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

async function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={['/skills/reorder-toothpaste']}>
          <Routes>
            <Route path="/skills/:name" element={<SkillDetail />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>,
    )
  })
  for (let tick = 0; tick < 5; tick += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
}

function text(): string {
  return (container.textContent ?? '').replace(/\s+/g, ' ')
}

function link(label: string): Element | undefined {
  return [...container.querySelectorAll('a')].find(
    (candidate) => (candidate.textContent ?? '').trim() === label,
  )
}

describe('the skill page without ultrafast', () => {
  it('tests that a skill that changes a site shows the skills picture and the way to ultrafast', async () => {
    ultrafast = { ...ultrafast, locked: true }
    await mount()

    expect(text()).toContain('ultrafast is off on this computer')
    expect(text()).toContain('Reorder my usual toothpaste')
    expect(text()).toContain(
      'Illustration. You pick with or without your agent.',
    )
    expect(link('See ultrafast')?.getAttribute('href')).toBe('/pro')
  })

  it('tests that the picture stays away once ultrafast is on', async () => {
    await mount()

    expect(text()).toContain('Reorder my usual toothpaste')
    expect(text()).not.toContain('Illustration.')
    expect(link('See ultrafast')).toBeUndefined()
  })
})
