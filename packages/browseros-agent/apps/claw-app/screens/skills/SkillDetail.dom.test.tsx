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
  jest,
  mock,
  setSystemTime,
} from 'bun:test'
import type {
  SkillDetail as Detail,
  Plan,
  Skill,
  SkillRun,
  SkillUltrafast,
} from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, createContext, type ReactNode, useContext, useState } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes } from 'react-router'

const api = await import('@/modules/api/client')
const posthog = await import('@/modules/analytics/posthog')

const captured: { event: string; properties?: Record<string, unknown> }[] = []
let capturing = true
const captureListeners = new Set<() => void>()
mock.module('@/modules/analytics/posthog', () => ({
  ...posthog,
  isCapturing: () => capturing,
  subscribeToCaptureState: (listener: () => void) => {
    captureListeners.add(listener)
    return () => captureListeners.delete(listener)
  },
  capture: (event: string, properties?: Record<string, unknown>) => {
    if (capturing) captured.push({ event, properties })
  },
}))

let ultrafast: SkillUltrafast
let overrides: Partial<Skill> = {}
let features: string[]
let runs: SkillRun[]
let clipboard: string[]
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
  runs,
  tokenSavings: { saved: 0, otherBrowsers: 0, used: 0, measuredRunCount: 0 },
})

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getSkill: async () => {
      duringFetch()
      return detail()
    },
    listConnections: async () => ({
      items: [{ harness: 'Claude Code', installed: true }],
    }),
    getPlan: async (): Promise<Plan> => ({
      available: true,
      plan: features.length ? 'ultrafast' : null,
      planName: features.length ? 'ultrafast' : null,
      features,
      status: features.length ? 'active' : 'none',
      periodEnd: null,
      endsAt: null,
      graceUntil: null,
      onKey: false,
      checkedAt: null,
      stale: false,
    }),
  }),
}))

// These pull in their own server calls and animated menus, which this page
// test does not exercise.
mock.module('@/components/skills/SkillActions', () => ({
  SkillActions: () => null,
}))
mock.module('@/components/skills/RunsChoiceDialog', () => ({
  RunsChoiceDialog: () => null,
}))
const Fold = createContext<{ open: boolean; toggle: () => void }>({
  open: false,
  toggle: () => {},
})

// The beUI collapsible animates its height; a plain one keeps the open state and the content.
mock.module('@/components/motion/collapsible', () => ({
  Collapsible: ({
    open,
    onOpenChange,
    children,
  }: {
    open?: boolean
    onOpenChange?: (open: boolean) => void
    children: ReactNode
  }) => {
    const [own, setOwn] = useState(false)
    const shown = open ?? own
    return (
      <Fold.Provider
        value={{
          open: shown,
          toggle: () => {
            setOwn(!shown)
            onOpenChange?.(!shown)
          },
        }}
      >
        <div>{children}</div>
      </Fold.Provider>
    )
  },
  CollapsibleTrigger: ({ children }: { children: ReactNode }) => {
    const fold = useContext(Fold)
    return (
      <button type="button" aria-expanded={fold.open} onClick={fold.toggle}>
        {children}
      </button>
    )
  },
  CollapsibleIndicator: () => null,
  CollapsibleContent: ({ children }: { children: ReactNode }) =>
    useContext(Fold).open ? <div>{children}</div> : null,
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
  captured.length = 0
  capturing = true
  overrides = {}
  features = ['claw.ultrafast']
  runs = []
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
  clipboard = []
  define('navigator', {
    userAgent: 'test',
    clipboard: {
      writeText: async (value: string) => {
        clipboard.push(value)
      },
    },
  })
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

async function openMore() {
  const trigger = [...container.querySelectorAll('button')].find((candidate) =>
    (candidate.textContent ?? '').startsWith('More about this skill'),
  )
  if (!trigger) throw new Error('no fold')
  await act(async () => {
    trigger.dispatchEvent(new Event('click', { bubbles: true }))
  })
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
    expect(text()).not.toContain('ultrafast is off on this computer')
    await openMore()

    expect(text()).toContain('ultrafast is off on this computer')
    expect(text()).toContain('Reorder my usual toothpaste')
    expect(text()).toContain(
      'Illustration. You pick with or without your agent.',
    )
    expect(link('See ultrafast')?.getAttribute('href')).toBe('/pro')
  })

  it('tests that the picture stays away once ultrafast is on', async () => {
    await mount()
    await openMore()

    expect(text()).toContain('Reorder my usual toothpaste')
    expect(text()).not.toContain('Illustration.')
    expect(link('See ultrafast')).toBeUndefined()
  })
})

describe('the skill page on a free plan', () => {
  function speedNote(): string | undefined {
    return [...container.querySelectorAll('p')]
      .map((line) => (line.textContent ?? '').replace(/\s+/g, ' ').trim())
      .find((line) => line.startsWith('Runs at normal speed'))
  }

  it('tests that the page says the skill runs at normal speed, with its usual time', async () => {
    features = []
    runs = [
      {
        id: 'r1',
        skillName: 'reorder-toothpaste',
        sessionId: 's1',
        runNumber: 1,
        agentId: 'claude',
        createdAt: 0,
        clean: true,
        mode: 'agent',
        durationMs: 41_000,
      },
    ]
    await mount()

    expect(speedNote()).toContain(
      'Runs at normal speed, about 41 sec a run. With ultrafast, neo runs it in seconds.',
    )
    expect(speedNote()).toContain('See ultrafast')
    expect(speedNote()).not.toContain('your agent')
  })

  it('tests that an ultrafast plan gets no speed note', async () => {
    await mount()

    expect(speedNote()).toBeUndefined()
  })
})

describe('the skill page with ultrafast', () => {
  function heading(): string | undefined {
    return [...container.querySelectorAll('h3')]
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

    expect(text()).toContain('Choose how it runs')
    await openMore()
    expect(heading()).toBe('ultrafast')
    expect(text()).toContain('about 1.2 s a run')
    expect(text()).toContain(
      'You can choose how it runs at the top of this page.',
    )
    expect(text()).not.toContain('Ultrafast')
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

    expect(text()).toContain('ultrafast needs a repair. The site changed.')
    await openMore()
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
    await openMore()

    expect(text()).not.toContain('Learning from this run')
    expect(text()).toContain('Your agent is turning this skill')
  })
})

describe('using a skill from its page', () => {
  /** The value row's figures, as "figure: what it is". */
  function tiles(): string[] {
    return [...container.querySelectorAll('dl > div')].map(
      (tile) =>
        `${(tile.querySelector('dd')?.textContent ?? '').trim()}: ${(tile.querySelector('dt')?.textContent ?? '').trim()}`,
    )
  }

  function copyBox(): Element | undefined {
    return [...container.querySelectorAll('button[aria-label]')].find(
      (candidate) =>
        candidate.getAttribute('aria-label') ===
        'Copy use the reorder-toothpaste skill',
    )
  }

  function spoken(): string {
    return [...container.querySelectorAll('[aria-live="polite"]')]
      .map((region) => (region.textContent ?? '').trim())
      .join(' | ')
  }

  it('tests that the copy box copies the phrase and says to paste it', async () => {
    await mount()
    expect(spoken()).toContain('Then paste it into any of your agents')
    expect(text()).toContain('Claude Code')

    await act(async () => {
      copyBox()?.dispatchEvent(new Event('click', { bubbles: true }))
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(clipboard).toEqual(['use the reorder-toothpaste skill'])
    expect(spoken()).toContain('Copied. Now paste it into your agent and send')
    expect(spoken()).not.toContain('Then paste it into any of your agents')
  })

  it('tests that the copy box goes back to its first line a few seconds after a copy', async () => {
    await mount()
    jest.useFakeTimers()
    try {
      await act(async () => {
        copyBox()?.dispatchEvent(new Event('click', { bubbles: true }))
      })
      await act(async () => {
        await Promise.resolve()
      })
      // The state comes from the copy itself, not an animation event, so reduced motion gets it.
      expect(spoken()).toContain(
        'Copied. Now paste it into your agent and send',
      )

      await act(async () => {
        jest.advanceTimersByTime(6000)
      })
      expect(spoken()).toContain('Then paste it into any of your agents')
    } finally {
      jest.useRealTimers()
    }
  })

  it('tests that the rest waits in a closed fold that opens to the steps', async () => {
    await mount()
    expect(text()).toContain('More about this skill')
    expect(text()).not.toContain('Steps neo follows')

    await openMore()
    expect(text()).toContain('Steps neo follows')
    expect(text()).toContain('Recent runs')
  })

  it('tests that a skill that runs the fast way a lot shows its speed', async () => {
    overrides = { runCount: 12, lastRunAt: Date.now() }
    ultrafast = {
      status: 'verified',
      flowVersion: 1,
      effects: 'read',
      writeApproved: false,
      p50Ms: 1800,
    }
    await mount()

    expect(tiles()).toEqual([
      '1.8s: a run',
      '12runs: so far',
      'today: last used',
    ])
  })

  it('tests that a skill that runs with the agent shows runs only', async () => {
    overrides = { runCount: 4, lastRunAt: Date.now() }
    ultrafast = {
      ...ultrafast,
      runs: {
        mode: 'with_agent',
        chosen: true,
        reask: false,
        click: 'Add it to the cart.',
        site: 'shop.example.com',
      },
    }
    await mount()

    expect(tiles()).toEqual(['4runs: last used today'])
  })

  it('tests that a free plan sees no wordmark up top, one speed note, and the off state in the fold', async () => {
    features = []
    ultrafast = { ...ultrafast, locked: true }
    await mount()

    const notes = [...container.querySelectorAll('p')].filter((line) =>
      (line.textContent ?? '').startsWith('Runs at normal speed'),
    )
    expect(notes).toHaveLength(1)
    const header = container.querySelector('header')
    expect(header?.textContent).not.toContain('ultrafast')
    expect(text()).not.toContain('ultrafast is off on this computer')

    await openMore()
    expect(text()).toContain('ultrafast is off on this computer')
  })
})

describe('counting skill page views', () => {
  const views = () =>
    captured
      .filter((entry) => entry.event === 'skill_page_viewed')
      .map((entry) => entry.properties)

  it('tests that opening a skill counts one view with how much it is used and its ultrafast state', async () => {
    overrides = { runCount: 12, lastRunAt: Date.now() }
    await mount()
    await mount()

    expect(views()).toEqual([
      { usage_level: 'lot', ultrafast_state: 'verified' },
    ])
  })

  it('tests that a skill on a free plan counts as locked, and a new one as never used', async () => {
    features = []
    overrides = { runCount: 0 }
    ultrafast = { ...ultrafast, locked: true }
    await mount()

    expect(views()).toEqual([
      { usage_level: 'never', ultrafast_state: 'locked' },
    ])
  })

  it('tests that a deep link opened before analytics starts still counts one view once it does', async () => {
    capturing = false
    await mount()
    expect(views()).toEqual([])

    capturing = true
    await act(async () => {
      for (const listener of captureListeners) listener()
    })

    expect(views()).toEqual([
      { usage_level: 'sometimes', ultrafast_state: 'verified' },
    ])
  })
})
