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
import type { Plan, Skill } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import {
  act,
  cloneElement,
  createContext,
  type ReactElement,
  type ReactNode,
  useContext,
} from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes } from 'react-router'

const api = await import('@/modules/api/client')

let features: string[]
let available: boolean
let skills: Skill[]

const plan = (): Plan => ({
  available,
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
})

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getPlan: async () => plan(),
    listSkills: async () => ({ items: skills }),
    listConnections: async () => ({ items: [] }),
  }),
}))

const ModalOpen = createContext<{
  open: boolean
  setOpen: (open: boolean) => void
}>({ open: false, setOpen: () => undefined })

// The beUI modal animates and measures the DOM, which linkedom cannot do; a plain dialog keeps
// the open state.
mock.module('@/components/motion/center-morph-modal', () => ({
  CenterMorphModal: ({
    open,
    onOpenChange,
    children,
  }: {
    open?: boolean
    onOpenChange?: (open: boolean) => void
    children: ReactNode
  }) => (
    <ModalOpen.Provider
      value={{ open: Boolean(open), setOpen: (next) => onOpenChange?.(next) }}
    >
      {children}
    </ModalOpen.Provider>
  ),
  CenterMorphModalContent: ({
    ariaLabel,
    children,
  }: {
    ariaLabel: string
    children: ReactNode
  }) =>
    useContext(ModalOpen).open ? (
      <div role="dialog" aria-label={ariaLabel}>
        {children}
      </div>
    ) : null,
  CenterMorphModalClose: ({
    children,
  }: {
    children: ReactElement<{ onClick?: () => void }>
  }) => {
    const modal = useContext(ModalOpen)
    return cloneElement(children, {
      onClick: () => {
        children.props.onClick?.()
        modal.setOpen(false)
      },
    })
  },
}))
mock.module('@/components/skills/SkillActions', () => ({
  SkillActions: () => null,
}))
mock.module('@/components/skills/SkillFormDialog', () => ({
  SkillFormDialog: () => null,
}))

const { Skills } = await import('./Skills')

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

function skill(index: number, overrides: Partial<Skill> = {}): Skill {
  return {
    name: `neo-skill-${index}`,
    description: `Skill number ${index}`,
    site: 'example.com',
    origin: 'agent',
    version: 1,
    linkedAgents: [],
    runCount: 0,
    cleanRunCount: 0,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  }
}

function many(count: number): Skill[] {
  return Array.from({ length: count }, (_, index) => skill(index + 1))
}

beforeEach(async () => {
  features = []
  available = true
  skills = many(2)
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
        <MemoryRouter initialEntries={['/skills']}>
          <Routes>
            <Route path="/skills" element={<Skills />} />
            <Route path="/pro" element={<p>ultrafast page</p>} />
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

function buttons(label: string): Element[] {
  return [...container.querySelectorAll('button, a')].filter(
    (candidate) => (candidate.textContent ?? '').trim() === label,
  )
}

async function click(target: Element | undefined) {
  if (!target) throw new Error('nothing to click')
  await act(async () => {
    target.dispatchEvent(new Event('click', { bubbles: true }))
  })
}

const SPEED_LINE =
  'Your skills run at normal speed. With ultrafast, neo runs them in seconds.'

describe('the Skills list on a free plan', () => {
  for (const count of [0, 2, 24]) {
    it(`tests that the speed line shows with ${count} skills`, async () => {
      skills = many(count)
      await mount()

      expect(text()).toContain(SPEED_LINE)
      expect(buttons('See ultrafast')).toHaveLength(1)
    })
  }

  it('tests that See ultrafast opens the dialog that sells speed', async () => {
    await mount()
    await click(buttons('See ultrafast')[0])

    const dialog = container.querySelector('[role="dialog"]')
    expect(dialog?.getAttribute('aria-label')).toBe(
      'Run your skills in seconds',
    )
    expect(dialog?.textContent).toContain('Your skills run in seconds')
    expect(buttons('Get ultrafast')[0]?.getAttribute('href') ?? undefined).toBe(
      '/pro',
    )
  })
})

describe('the Skills list without the speed line', () => {
  it('tests that an ultrafast plan does not hear about ultrafast', async () => {
    features = ['claw.ultrafast']
    await mount()

    expect(text()).not.toContain('normal speed')
    expect(buttons('See ultrafast')).toHaveLength(0)
  })

  it('tests that a build without ultrafast on sale shows no line', async () => {
    available = false
    await mount()

    expect(text()).not.toContain('normal speed')
  })
})
