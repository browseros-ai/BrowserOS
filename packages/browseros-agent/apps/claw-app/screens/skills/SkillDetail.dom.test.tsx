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
  setSystemTime,
} from 'bun:test'
import type {
  SkillDetail as Detail,
  Skill,
  SkillUltrafast,
} from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes } from 'react-router'

const api = await import('@/modules/api/client')

let ultrafast: SkillUltrafast
let overrides: Partial<Skill> = {}
// Runs inside each fetch, so a test can move the clock while the page waits on the server.
let duringFetch = () => {}

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
    ...overrides,
  },
  body: '',
  runs: [],
  tokenSavings: { saved: 0, otherBrowsers: 0, used: 0, measuredRunCount: 0 },
})

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getSkill: async () => {
      duringFetch()
      return detail()
    },
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
  overrides = {}
  duringFetch = () => {}
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
  setSystemTime()
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

describe('the skill page with ultrafast', () => {
  function heading(): string | undefined {
    return [...container.querySelectorAll('h2')]
      .map((candidate) => (candidate.textContent ?? '').trim())
      .find((candidate) => candidate.startsWith('ultrafast'))
  }

  it('tests that a verified skill names ultrafast and its speed, and asks how it runs', async () => {
    ultrafast = {
      ...ultrafast,
      runs: {
        mode: 'with_agent',
        chosen: false,
        reask: false,
        click: 'Add it to the cart.',
        site: 'shop.example.com',
      },
    }
    await mount()

    expect(heading()).toBe('ultrafast')
    expect(text()).toContain('ultrafast about 1.2 s a run')
    expect(text()).not.toContain('Ultrafast')
    expect(text()).toContain('Choose how it runs')
    expect(text()).toContain('Without your agent ultrafast')
    const names = [...container.querySelectorAll('[aria-labelledby]')].map(
      (radio) =>
        document.getElementById(radio.getAttribute('aria-labelledby') ?? '')
          ?.textContent,
    )
    expect(names).toEqual(['With your agent', 'Without your agent'])
  })

  it('tests that a skill that needs a repair says so beside the wordmark', async () => {
    ultrafast = { ...ultrafast, status: 'degraded' }
    await mount()

    expect(heading()).toBe('ultrafast needs a repair')
  })
})

describe('the first run of a skill made from a task', () => {
  it('tests that the page says what neo is doing while it checks the fast version', async () => {
    overrides = { origin: 'history', runCount: 0 }
    ultrafast = {
      status: 'pending',
      flowVersion: 0,
      writeApproved: false,
    }
    await mount()

    expect(text()).toContain('Learning from this run')
    expect(text()).toContain(
      'Done: Your agent is doing the task the normal way',
    )
    expect(text()).toContain('Done: neo is learning the steps')
    expect(text()).toContain('In progress: neo is checking the fast version')
    expect(text()).toContain('Still to come: Next time it takes seconds')
    expect(text()).not.toContain('Your agent is turning this skill')
  })

  it('tests that the page stops saying neo is checking after two minutes', async () => {
    overrides = { origin: 'history', runCount: 0 }
    ultrafast = { status: 'pending', flowVersion: 0, writeApproved: false }
    duringFetch = () => setSystemTime(new Date(Date.now() + 3 * 60_000))
    await mount()

    expect(text()).toContain('Learning from this run')
    expect(text()).not.toContain('neo is checking the fast version')
    expect(text()).toContain('Still to come: Finishes on its next run')
  })

  it('tests that a verified task skill shows its fast version instead', async () => {
    overrides = { origin: 'history', runCount: 1 }
    ultrafast = { ...ultrafast, status: 'verified', verifiedAt: 1 }
    await mount()

    expect(text()).not.toContain('Learning from this run')
  })

  it('tests that a skill an agent saved never shows the panel', async () => {
    ultrafast = { status: 'pending', flowVersion: 0, writeApproved: false }
    await mount()

    expect(text()).not.toContain('Learning from this run')
    expect(text()).toContain('Your agent is turning this skill')
  })
})
