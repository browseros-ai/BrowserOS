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
let clipboard: string[]

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
    listConnections: async () => ({
      items: [
        { harness: 'Claude Code', installed: true },
        { harness: 'Codex', installed: true },
      ],
    }),
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
  SkillActions: ({ skill }: { skill: Skill }) => (
    <button type="button">More for {skill.description}</button>
  ),
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
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

let client: QueryClient

async function mount() {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={['/skills']}>
          <Routes>
            <Route path="/skills" element={<Skills />} />
            <Route path="/skills/:name" element={<p>A skill page</p>} />
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

/** Types into the search box, which reads what was typed from each input event. */
async function typeInto(input: Element, value: string) {
  await act(async () => {
    ;(input as HTMLInputElement).value = value
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function card(title: string): Element | undefined {
  return [...container.querySelectorAll('button[aria-label]')].find(
    (candidate) =>
      candidate.getAttribute('aria-label') === `Copy skill: ${title}`,
  )
}

describe('copying a skill from the list', () => {
  it('tests that clicking a card copies its phrase and says to paste it', async () => {
    features = ['claw.ultrafast']
    await mount()
    expect(text()).not.toContain('Paste it into your agent')

    await click(card('Skill number 2'))
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(clipboard).toEqual(['use the neo-skill-2 skill'])
    expect(text()).toContain('Paste it into your agent')
  })

  it('tests that a copied card shows Copied and the paste hint, then its stats again', async () => {
    features = ['claw.ultrafast']
    await mount()
    jest.useFakeTimers()
    try {
      await click(card('Skill number 2'))
      await act(async () => {
        await Promise.resolve()
      })
      // No animation or transition event is needed for the state, so reduced motion still gets it.
      expect(text()).toContain('Copied')
      expect(text()).toContain('Paste it into your agent')

      await act(async () => {
        jest.advanceTimersByTime(6000)
      })
      // The hint stays mounted while it fades out, then leaves.
      expect(text()).toContain('Paste it into your agent')
      await act(async () => {
        jest.advanceTimersByTime(400)
      })
      expect(text()).not.toContain('Paste it into your agent')
      expect(text()).toContain('Details')
    } finally {
      jest.useRealTimers()
    }
  })

  it('tests that Details links to the skill and the menu does not copy', async () => {
    await mount()
    await click(buttons('More for Skill number 1')[0])
    expect(clipboard).toEqual([])

    const details = container.querySelector('a[href="/skills/neo-skill-1"]')
    expect(details?.textContent?.trim()).toBe('Details')
  })

  it('tests that a long list can be searched by title, name or site', async () => {
    skills = [
      ...many(22),
      skill(23, {
        description: 'Hacker News top 10',
        site: 'news.ycombinator.com',
      }),
      skill(24, {
        description: 'Read the top stories',
        site: 'news.google.com',
      }),
    ]
    await mount()
    expect(text()).toContain('24 skills')

    const search = container.querySelector('input[type="search"]')
    if (!search) throw new Error('no search box')
    await typeInto(search, 'news')
    expect(text()).toContain('2 of 24 skills')
    expect(card('Hacker News top 10')).toBeDefined()
    expect(card('Skill number 1')).toBeUndefined()
  })

  it('tests that a short list has no search', async () => {
    skills = many(3)
    await mount()
    expect(container.querySelector('input[type="search"]')).toBeNull()
  })

  it('tests that no skills shows what a skill is and how to save one', async () => {
    skills = []
    features = ['claw.ultrafast']
    await mount()

    expect(text()).toContain('No skills yet')
    expect(text()).toContain('Then tell it')
    expect(buttons('Make one by hand')).toHaveLength(1)
    await click(
      container.querySelector(
        'button[aria-label="Copy save this as a skill in neo"]',
      ) ?? undefined,
    )
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(clipboard).toEqual(['save this as a skill in neo'])
    expect(text()).toContain('Copied. Paste it after your next task')
  })

  it('tests that a free plan shows no ultrafast marks on its skills', async () => {
    skills = [
      skill(1, {
        origin: 'history',
        runCount: 4,
        ultrafast: {
          status: 'verified',
          flowVersion: 1,
          writeApproved: false,
          p50Ms: 1800,
          locked: true,
        },
      }),
    ]
    await mount()

    expect(text()).toContain('4 runs')
    const marks = [...container.querySelectorAll('li span')].filter(
      (span) => (span.textContent ?? '').trim() === 'ultrafast',
    )
    expect(marks).toHaveLength(0)
  })
})

describe('searching while the list shrinks', () => {
  it('tests that the search box stays while a search is active, and clearing it shows every skill', async () => {
    skills = many(7)
    await mount()
    const search = () => container.querySelector('input[type="search"]')
    await typeInto(search() as Element, 'number 7')
    expect(text()).toContain('1 of 7 skills')

    skills = many(6)
    await act(async () => {
      await client.invalidateQueries()
    })
    for (let tick = 0; tick < 3; tick += 1) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0))
      })
    }
    expect(search()).not.toBeNull()
    expect((search() as HTMLInputElement).value).toBe('number 7')
    expect(text()).toContain('No skills match “number 7”.')

    await typeInto(search() as Element, '')
    expect(card('Skill number 1')).toBeDefined()
    expect(card('Skill number 6')).toBeDefined()
    expect(search()).toBeNull()
  })
})

describe('the paste hint after a copy', () => {
  it('tests that a skill linked only to Codex names only Codex', async () => {
    skills = [
      skill(1, { linkedAgents: ['Codex'] }),
      skill(2, { linkedAgents: ['Claude Code', 'Codex'] }),
    ]
    await mount()
    await click(card('Skill number 1'))
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })

    const faces = [...container.querySelectorAll('[role="img"]')].map((img) =>
      img.getAttribute('aria-label'),
    )
    expect(faces).toEqual(['Codex'])
  })
})
