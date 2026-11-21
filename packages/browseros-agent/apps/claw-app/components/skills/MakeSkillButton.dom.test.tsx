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
import type {
  Plan,
  SessionSummary,
  SkillCreate,
  SkillDraftStatus,
} from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes, useParams } from 'react-router'
import type { ToastInput } from '@/components/motion/animated-toast-stack'
import { connectToastHost } from '@/modules/toast/toast'

const api = await import('@/modules/api/client')
const posthog = await import('@/modules/analytics/posthog')

interface Captured {
  event: string
  properties?: Record<string, unknown>
}
/** Every event this file captured, kept across tests: impressions count once per app load. */
const everCaptured: Captured[] = []
let captured: Captured[]

mock.module('@/modules/analytics/posthog', () => ({
  ...posthog,
  capture: (event: string, properties?: Record<string, unknown>) => {
    everCaptured.push({ event, properties })
    captured.push({ event, properties })
  },
}))

let features: string[]
let draftReply: SkillDraftStatus
let draftCalls: number
let created: SkillCreate[]
let deleted: string[]

const ready: SkillDraftStatus = {
  status: 'ready',
  draftId: 'd1',
  expiresAt: 0,
  draft: {
    name: 'flipkart-phones-under-budget',
    title: 'Find phones under a budget on Flipkart',
    description: 'Lists the most popular phones under your budget.',
    site: 'flipkart.com',
    steps: [
      'Open flipkart.com and go to Mobiles.',
      'Set the price filter to a maximum of {budget}.',
    ],
    inputs: [
      {
        name: 'budget',
        label: 'Highest price',
        example: '₹30,000',
        kind: 'number',
      },
    ],
    notes: [],
    effects: 'read',
    confidence: 'high',
  },
}

const plan = (): Plan => ({
  available: true,
  plan: 'ultrafast',
  planName: 'ultrafast',
  features,
  status: 'active',
  periodEnd: null,
  endsAt: null,
  graceUntil: null,
  onKey: false,
  checkedAt: null,
  stale: false,
})

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getPlan: async () => plan(),
    getSkillDraft: async () => ({ status: 'none' }),
    draftSkillFromSession: async () => {
      draftCalls += 1
      await new Promise((resolve) => setTimeout(resolve, 5))
      return draftReply
    },
    createSkill: async (body: SkillCreate) => {
      created.push(body)
      return {
        name: body.name,
        description: body.description,
        origin: 'history',
        version: 1,
        linkedAgents: [],
        runCount: 0,
        cleanRunCount: 0,
        createdAt: 0,
        updatedAt: 0,
      }
    },
    deleteSkill: async ({ name }: { name: string }) => {
      deleted.push(name)
    },
    listSkills: async () => ({ items: [] }),
    listSessions: async () => ({ items: [] }),
  }),
}))

// The beUI modal animates through motion and measures the DOM, which linkedom cannot do. A plain
// dialog keeps the open state and the views.
mock.module('@/components/motion/morphing-modal', () => ({
  MorphingModal: ({
    viewId,
    children,
  }: {
    viewId: string | null
    children: ReactNode
  }) =>
    viewId ? (
      <div role="dialog" data-view={viewId}>
        {children}
      </div>
    ) : null,
}))
mock.module('@/components/motion/tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children: ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children: ReactNode }) => (
    <span role="tooltip">{children}</span>
  ),
}))
mock.module('@/components/copy/CopyButton', () => ({
  CopyButton: ({ text }: { text: string }) => (
    <button type="button">Copy {text}</button>
  ),
}))

const { MakeSkillButton } = await import('./MakeSkillButton')

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

function task(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: 'f392a113',
    slug: 'claude-code',
    label: 'Claude Code',
    name: 'Browsed www.flipkart.com',
    site: 'www.flipkart.com',
    startedAt: Date.now() - 60_000,
    durationMs: 300_000,
    dispatchCount: 12,
    toolSequence: [],
    status: 'done',
    errorCount: 0,
    ...overrides,
  }
}

beforeEach(async () => {
  features = ['claw.ultrafast']
  draftReply = ready
  draftCalls = 0
  created = []
  deleted = []
  captured = []
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

async function settle(ticks = 6) {
  for (let tick = 0; tick < ticks; tick += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5))
    })
  }
}

async function mount(summary: SessionSummary) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={['/']}>
          <Routes>
            <Route
              path="/"
              element={<MakeSkillButton task={summary} surface="page" />}
            />
            <Route path="/skills" element={<p>Skills page</p>} />
            <Route path="/skills/:name" element={<SkillPage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>,
    )
  })
  await settle()
}

function SkillPage() {
  const { name } = useParams()
  return <p>Skill page {name}</p>
}

function text(): string {
  return (container.textContent ?? '').replace(/\s+/g, ' ')
}

/** The button whose shown label (the one not hidden from assistive tech) is `label`. */
function button(label: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll('button')].find((candidate) => {
    const shown = candidate.querySelector('span:not([aria-hidden="true"])')
    return (shown?.textContent ?? candidate.textContent ?? '').trim() === label
  }) as HTMLButtonElement | undefined
}

async function click(target: Element | undefined) {
  if (!target) throw new Error('nothing to click')
  await act(async () => {
    target.dispatchEvent(new Event('click', { bubbles: true }))
  })
}

describe('making a skill from a task', () => {
  it('tests that a plan without ultrafast sees no button', async () => {
    features = []
    await mount(task())
    expect(button('Make it a skill')).toBeUndefined()
  })

  it('tests that a task stopped without errors gets the button', async () => {
    await mount(task({ status: 'cancelled', errorCount: 0 }))
    expect(button('Make it a skill')).toBeDefined()
  })

  it('tests that a task stopped after errors gets no button', async () => {
    await mount(task({ status: 'cancelled', errorCount: 1 }))
    expect(button('Make it a skill')).toBeUndefined()
  })

  it('tests that a task with a skill opens it instead', async () => {
    await mount(task({ skillName: 'neo-flipkart-phones-under-budget' }))
    expect(button('Open skill')).toBeDefined()
  })

  it('tests that a burst of clicks writes one draft, and saving sends the edits', async () => {
    await mount(task())
    const make = button('Make it a skill')
    await click(make)
    await click(make)
    await click(make)
    expect(text()).toContain('Writing your skill from this task.')
    await settle()

    expect(draftCalls).toBe(1)
    expect(text()).toContain('It starts as a regular skill.')
    expect(text()).toContain('use the neo-flipkart-phones-under-budget skill')
    expect(button('Checking your skill')).toBeDefined()

    expect(
      container.querySelector('textarea[aria-label="Step 2"]'),
    ).not.toBeNull()
    await click(
      container.querySelector('button[aria-label="Remove step 1"]') ??
        undefined,
    )
    // linkedom does not turn a submit button's click into a submit, so submit the form itself.
    const form = container.querySelector('[role="dialog"] form')
    await act(async () => {
      form?.dispatchEvent(
        new Event('submit', { bubbles: true, cancelable: true }),
      )
    })
    await settle()

    expect(created).toHaveLength(1)
    expect(created[0]?.sourceSessionId).toBe('f392a113')
    expect(created[0]?.name).toBe('neo-flipkart-phones-under-budget')
    expect(created[0]?.description).toBe(
      'Find phones under a budget on Flipkart',
    )
    expect(created[0]?.steps).toEqual([
      'Set the price filter to a maximum of {budget}.',
    ])
    expect(created[0]?.inputs?.map((input) => input.name)).toEqual(['budget'])
    expect(text()).toContain('Skill page neo-flipkart-phones-under-budget')

    const events = captured.filter(
      ({ event }) => event !== 'skill_from_task_cta_shown',
    )
    expect(events).toEqual([
      {
        event: 'skill_from_task_clicked',
        properties: { surface: 'task_page' },
      },
      {
        event: 'skill_draft_generated',
        properties: {
          ms: expect.any(Number),
          confidence: 'high',
          steps: 2,
          inputs: 1,
        },
      },
      {
        event: 'skill_from_task_saved',
        properties: {
          edited_name: false,
          edited_steps: true,
          edited_inputs: false,
          steps: 1,
          inputs: 1,
        },
      },
    ])
  })

  it('tests that undo after saving deletes the skill and goes back to the list', async () => {
    const shown: ToastInput[] = []
    const disconnect = connectToastHost({
      show: (input) => {
        shown.push(input)
        return String(shown.length)
      },
      update: () => {},
    })
    // Connecting shows toasts earlier tests queued with no host; only this test's count.
    shown.length = 0
    try {
      await mount(task())
      await click(button('Make it a skill'))
      await settle()
      const form = container.querySelector('[role="dialog"] form')
      await act(async () => {
        form?.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true }),
        )
      })
      await settle()
      expect(text()).toContain('Skill page neo-flipkart-phones-under-budget')

      const saved = shown.find(({ title }) => title === 'Skill saved.')
      if (!saved?.action) throw new Error('no undo on the saved toast')
      const undo = saved.action
      await act(async () => undo.onClick({ ...saved, id: '1' }))
      await settle()

      expect(deleted).toEqual(['neo-flipkart-phones-under-budget'])
      expect(text()).toContain('Skills page')
    } finally {
      disconnect()
    }
  })

  it('tests that the button counts as shown once per place, and closing a draft is counted', async () => {
    await mount(task())
    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    await mount(task())
    expect(
      everCaptured.filter(
        ({ event, properties }) =>
          event === 'skill_from_task_cta_shown' &&
          properties?.surface === 'task_page',
      ),
    ).toHaveLength(1)

    await click(button('Make it a skill'))
    await settle()
    await click(
      container.querySelector('[role="dialog"] button[aria-label="Close"]') ??
        undefined,
    )
    expect(captured).toContainEqual({
      event: 'skill_draft_discarded',
      properties: { stage: 'review' },
    })
  })

  it('tests that a limit explains itself and points to the agent', async () => {
    draftReply = {
      status: 'failed',
      draftId: 'd1',
      failure: {
        code: 'limit_reached',
        scope: 'day',
        resetsAt: new Date(Date.now() + 3_600_000).toISOString(),
      },
    }
    await mount(task())
    await click(button('Make it a skill'))
    await settle()

    expect(text()).toContain('You’ve made 100 skills today.')
    expect(text()).toContain('You can make more tomorrow.')
    expect(text()).toContain('Your agent can still save one for you.')
    expect(text()).toContain('save this as a skill in neo')
    expect(button('Try again')).toBeUndefined()
    expect(captured).toContainEqual({
      event: 'skill_draft_failed',
      properties: { code: 'limit_reached' },
    })
  })

  it('tests that a failed draft offers a retry', async () => {
    draftReply = {
      status: 'failed',
      draftId: 'd1',
      failure: { code: 'upstream_error' },
    }
    await mount(task())
    await click(button('Make it a skill'))
    await settle()

    expect(text()).toContain('We couldn’t write this skill right now.')
    draftReply = ready
    const retries = [
      ...container.querySelectorAll('[role="dialog"] button'),
    ].filter(
      (candidate) => (candidate.textContent ?? '').trim() === 'Try again',
    )
    await click(retries[0])
    await settle()
    expect(draftCalls).toBe(2)
    expect(text()).toContain('It starts as a regular skill.')
  })
})
