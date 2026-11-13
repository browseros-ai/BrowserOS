/**
 * The ultrafast page against a stubbed claw-server. The real react-query hooks
 * run; only the HTTP client, the analytics transport, the new-tab call and
 * the animated modals are replaced.
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
  ActivatePlanKeyRequest,
  CancelPlanRequest,
  Plan,
  PlanCheckoutStatus,
  PlanComputer,
  PlanKey,
  PlanKeyChange,
  PlanPortalRequest,
  PlanPricing,
  PlanSavings,
  SessionSummary,
  StartPlanCheckoutRequest,
} from '@browseros/claw-api'
import { ApiResponseError } from '@browseros/claw-api-client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import {
  act,
  cloneElement,
  createContext,
  type InputHTMLAttributes,
  type ReactElement,
  type ReactNode,
  type Ref,
  useContext,
} from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes } from 'react-router'
import type { ToastInput } from '@/components/motion/animated-toast-stack'
import * as posthog from '@/modules/analytics/posthog'
import * as api from '@/modules/api/client'
import { connectToastHost } from '@/modules/toast/toast'

const FULL_KEY = /NEO(-[0-9A-Z]{5}){5}/
const PASTED = 'NEO-7K2QM-R4X9B-T6WZP-3HDVN-8CJYF'
const PERIOD_END = new Date(2026, 10, 27, 12).getTime()
const GRACE_UNTIL = new Date(2026, 11, 4, 12).getTime()

const planOf = (overrides: Partial<Plan> = {}): Plan => ({
  available: true,
  plan: null,
  planName: null,
  features: [],
  status: 'none',
  periodEnd: null,
  endsAt: null,
  graceUntil: null,
  onKey: false,
  checkedAt: 1,
  stale: false,
  ...overrides,
})

const PRO = planOf({
  plan: 'ultrafast',
  planName: 'ultrafast',
  features: ['claw.ultrafast'],
  status: 'active',
  periodEnd: PERIOD_END,
  onKey: true,
})

const PRICING: PlanPricing = {
  plans: [
    {
      plan: 'ultrafast',
      name: 'ultrafast',
      description: null,
      features: ['claw.ultrafast'],
      prices: [
        { interval: 'month', intervalCount: 1, amount: 2000, currency: 'usd' },
        { interval: 'year', intervalCount: 1, amount: 20000, currency: 'usd' },
      ],
    },
  ],
}

const computer = (overrides: Partial<PlanComputer>): PlanComputer => ({
  installId: 'this-computer',
  label: 'Sam’s MacBook Air',
  platform: 'macos',
  lastSeenAt: Date.now(),
  activatedAt: new Date(2026, 9, 27, 9, 41).getTime(),
  thisComputer: true,
  ...overrides,
})

const THIS = computer({})
const OTHER = computer({
  installId: 'other-computer',
  label: 'DESKTOP-4FQ7K2L',
  platform: 'windows',
  thisComputer: false,
})
const KEY: PlanKey = { hint: '7K2QM', cap: 5, computers: [THIS, OTHER] }

const NO_SAVINGS: PlanSavings = {
  totals: { durationMs: 0, pagesRead: 0, tokensSaved: 0 },
  tasks: 0,
  latest: [],
}

const ranOn = (
  sessionId: string,
  name: string,
  ultrafast: SessionSummary['ultrafast'],
  pursue?: SessionSummary['pursue'],
): SessionSummary =>
  ({
    sessionId,
    name,
    slug: 'claude',
    label: 'Claude Code',
    status: 'done',
    startedAt: new Date(2026, 10, 13, 9, 5).getTime(),
    durationMs: 30_000,
    dispatchCount: 6,
    errorCount: 0,
    ultrafast,
    pursue,
  }) as SessionSummary

const SAVINGS: PlanSavings = {
  totals: { durationMs: 468_000, pagesRead: 141, tokensSaved: 972_900 },
  tasks: 23,
  latest: [
    ranOn('pursue-1', 'Search flights to Singapore', undefined, {
      outcome: 'handed_back',
      durationMs: 5_662,
      steps: [
        { action: 'click' },
        { action: 'click' },
        { action: 'type', text: 'Singapore' },
      ],
    }),
    ranOn(
      'pursue-2',
      'Search amazon.in for Colgate',
      { durationMs: 7_317, pagesRead: 5, tokensSaved: 18_204 },
      { outcome: 'completed', durationMs: 7_317, steps: [] },
    ),
    ranOn('skill-1', 'Today’s phone deals on Flipkart', {
      durationMs: 15_354,
      pagesRead: 9,
      tokensSaved: 61_942,
    }),
  ],
}

function apiError(status: number, body: Record<string, unknown>) {
  return new ApiResponseError(new Response(null, { status }), body)
}

interface Server {
  plan: () => Promise<Plan>
  pricing: () => Promise<PlanPricing>
  key: () => Promise<PlanKey>
  savings: () => Promise<PlanSavings>
  startCheckout: (body: StartPlanCheckoutRequest) => Promise<unknown>
  checkout: () => Promise<PlanCheckoutStatus>
  activate: (body: ActivatePlanKeyRequest) => Promise<PlanKeyChange>
  remove: (installId: string) => Promise<PlanKeyChange>
  rename: (installId: string, name: string) => Promise<PlanKey>
  reset: () => Promise<PlanKeyChange>
  cancel: (body: CancelPlanRequest) => Promise<Plan>
  resume: () => Promise<Plan>
  portal: (body: PlanPortalRequest) => Promise<{ url: string }>
  calls: { name: string; body?: unknown }[]
}

const server = {} as Server

function resetServer() {
  Object.assign(server, {
    plan: async () => planOf(),
    pricing: async () => PRICING,
    key: async () => KEY,
    savings: async () => NO_SAVINGS,
    startCheckout: async () => ({
      checkoutId: 'checkout-1',
      url: 'https://checkout.stripe.com/c/pay/cs_test_1',
    }),
    checkout: async () => ({ status: 'open', plan: null }),
    activate: async () => ({ plan: PRO, key: KEY }),
    remove: async () => ({ plan: PRO, key: { ...KEY, computers: [THIS] } }),
    rename: async (_installId: string, name: string) => ({
      ...KEY,
      computers: [THIS, { ...OTHER, label: name }],
    }),
    reset: async () => ({ plan: PRO, key: { ...KEY, hint: 'Q3VTM' } }),
    cancel: async () =>
      ({ ...PRO, status: 'ending', endsAt: PERIOD_END }) satisfies Plan,
    resume: async () => PRO,
    portal: async () => ({ url: 'https://billing.stripe.com/p/session/1' }),
    calls: [],
  } satisfies Server)
}

const record = <T,>(name: string, body: unknown, run: () => Promise<T>) => {
  server.calls.push({ name, body })
  return run()
}

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getPlan: () => record('getPlan', undefined, server.plan),
    getPlanPricing: () => record('getPlanPricing', undefined, server.pricing),
    getPlanKey: () => record('getPlanKey', undefined, server.key),
    getPlanSavings: () => record('getPlanSavings', undefined, server.savings),
    startPlanCheckout: (body: StartPlanCheckoutRequest) =>
      record('startPlanCheckout', body, () => server.startCheckout(body)),
    getPlanCheckout: () =>
      record('getPlanCheckout', undefined, server.checkout),
    activatePlanKey: (body: ActivatePlanKeyRequest) =>
      record('activatePlanKey', body, () => server.activate(body)),
    removePlanComputer: ({ installId }: { installId: string }) =>
      record('removePlanComputer', installId, () => server.remove(installId)),
    renamePlanComputer: ({
      installId,
      name,
    }: {
      installId: string
      name: string
    }) =>
      record('renamePlanComputer', { installId, name }, () =>
        server.rename(installId, name),
      ),
    resetPlanKey: () => record('resetPlanKey', undefined, server.reset),
    cancelPlan: (body: CancelPlanRequest) =>
      record('cancelPlan', body, () => server.cancel(body)),
    resumePlan: () => record('resumePlan', undefined, server.resume),
    openPlanPortal: (body: PlanPortalRequest) =>
      record('openPlanPortal', body, () => server.portal(body)),
    refreshPlan: () => record('refreshPlan', undefined, server.plan),
  }),
}))

const captured: { event: string; properties?: Record<string, unknown> }[] = []
mock.module('@/modules/analytics/posthog', () => ({
  ...posthog,
  capture: (event: string, properties?: Record<string, unknown>) => {
    captured.push({ event, properties })
  },
}))

// The beUI modals animate through motion and measure the DOM, which linkedom
// cannot do. Plain dialogs keep the open state, the names and the close wiring.
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

// The beUI field shakes on an error through motion's DOM animation, which
// linkedom cannot run. A plain field keeps its label, value, ref and error line.
mock.module('@/components/motion/input', () => ({
  Input: ({
    error,
    classNames: _classNames,
    defaultValue,
    ref,
    ...rest
  }: InputHTMLAttributes<HTMLInputElement> & {
    error?: string
    classNames?: unknown
    defaultValue?: string
    ref?: Ref<HTMLInputElement>
  }) => (
    <div>
      <input
        ref={ref}
        defaultValue={defaultValue}
        aria-invalid={error ? true : undefined}
        {...rest}
      />
      {error ? <p role="alert">{error}</p> : null}
    </div>
  ),
}))

const ModalOpen = createContext<{
  open: boolean
  setOpen: (open: boolean) => void
}>({ open: false, setOpen: () => undefined })

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
      value={{
        open: Boolean(open),
        setOpen: (next) => onOpenChange?.(next),
      }}
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
  }) => {
    const modal = useContext(ModalOpen)
    return modal.open ? (
      <div role="dialog" aria-label={ariaLabel}>
        {children}
      </div>
    ) : null
  },
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

const opened: string[] = []
const toasts: ToastInput[] = []

const { Pro } = await import('./Pro')

afterAll(() => mock.restore())

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'HTMLInputElement',
    'Node',
    'Event',
    'ResizeObserver',
    'getComputedStyle',
    'requestAnimationFrame',
    'cancelAnimationFrame',
    'chrome',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

let root: Root
let container: HTMLElement
let client: QueryClient
let disconnectToasts: () => void

beforeEach(async () => {
  resetServer()
  captured.length = 0
  opened.length = 0
  toasts.length = 0
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    HTMLInputElement: dom.window.HTMLInputElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    // linkedom has no layout or frames; the period tabs and the FAQ measure.
    ResizeObserver: class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
    getComputedStyle: () => ({ getPropertyValue: () => '' }),
    requestAnimationFrame: (run: FrameRequestCallback) =>
      setTimeout(() => run(0), 0),
    cancelAnimationFrame: (id: number) => clearTimeout(id),
    chrome: {
      tabs: {
        create: async ({ url }: { url: string }) => {
          opened.push(url)
        },
      },
    },
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  // React checks for oninput once, when react-dom loads; without it, React
  // treats key presses as possible edits and trips over linkedom's events.
  Object.defineProperty(dom.document, 'oninput', {
    configurable: true,
    writable: true,
    value: null,
  })
  container = dom.document.getElementById('root') as unknown as HTMLElement
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  disconnectToasts = connectToastHost({
    show: (input) => {
      toasts.push(input)
      return String(toasts.length)
    },
    update: () => undefined,
  })
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  disconnectToasts()
  client.clear()
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

// react-query hands results to React on 0 ms timers; let them run in act.
async function settle() {
  for (let i = 0; i < 4; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
}

async function mount() {
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={['/pro']}>
          <Routes>
            <Route path="/pro" element={<Pro />} />
            <Route path="/" element={<p>Home page</p>} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>,
    )
  })
  await settle()
}

function text(): string {
  return (container.textContent ?? '').replace(/\s/g, ' ')
}

/** The name a screen reader gives the element: its label, or its text minus hidden parts. */
function nameOf(element: Element): string {
  const label = element.getAttribute('aria-label')
  if (label) return label
  const copy = element.cloneNode(true) as Element
  for (const hidden of copy.querySelectorAll('[aria-hidden="true"]')) {
    hidden.remove()
  }
  return (copy.textContent ?? '').trim()
}

function button(label: string, within: Element = container): HTMLElement {
  const buttons = [...within.querySelectorAll('button')]
  const match = buttons.find((candidate) => nameOf(candidate) === label)
  if (!match) {
    throw new Error(
      `no "${label}" button among: ${buttons.map((b) => `"${nameOf(b)}"`).join(', ')}`,
    )
  }
  return match as unknown as HTMLElement
}

function dialog(): Element {
  const found = container.querySelector('[role="dialog"]')
  if (!found) throw new Error('no dialog open')
  return found
}

async function click(label: string, within?: Element) {
  const target = button(label, within)
  await act(async () => {
    target.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
  await settle()
}

function field(label: string): HTMLInputElement {
  const byLabel = [...container.querySelectorAll('label')].find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  const id = byLabel?.getAttribute('for')
  const input = id
    ? container.querySelector(`#${id}`)
    : container.querySelector(`[aria-label="${label}"]`)
  if (!input) throw new Error(`no "${label}" field`)
  return input as unknown as HTMLInputElement
}

// The field is a plain uncontrolled input here, so setting its value is all typing does.
async function type(input: HTMLInputElement, value: string) {
  input.value = value
}

async function pressKey(input: HTMLElement, key: string) {
  await act(async () => {
    const event = new window.Event('keydown', { bubbles: true })
    Object.defineProperty(event, 'key', { value: key })
    input.dispatchEvent(event)
  })
  await settle()
}

function callsTo(name: string) {
  return server.calls.filter((call) => call.name === name)
}

function events(name: string) {
  return captured.filter((entry) => entry.event === name)
}

function expectNoKeyAnywhere() {
  expect(text()).not.toMatch(FULL_KEY)
  for (const input of container.querySelectorAll('input')) {
    expect((input as unknown as HTMLInputElement).value).not.toMatch(FULL_KEY)
  }
  expect(JSON.stringify(captured)).not.toMatch(FULL_KEY)
}

describe('the ultrafast page', () => {
  it('tests that Free shows the monthly price, the way in with a key, and the questions', async () => {
    await mount()

    expect(text()).toContain('More from your AI agents.')
    expect(text()).toContain(
      'Illustration. Each dot is a step your agent pays for.',
    )
    expect(text()).toContain('Skills run in seconds')
    expect(text()).toContain('Robot checks, solved for you')
    expect(text()).not.toContain('Fewer steps. Smaller AI bills.')
    expect(text()).not.toContain('Show me')
    expect(text()).toContain('$20a month')
    expect(text()).toContain('Cancel any time.')
    expect(text()).toContain('2 months free')
    expect(button('Get ultrafast')).toBeTruthy()
    expect(button('Use a licence key')).toBeTruthy()
    expect(text()).toContain('Make your agents ultrafast')
    expect(text()).toContain('Questions')
    expect(button('Who sees my card?')).toBeTruthy()
    expect(button('Does neo change sites without asking?')).toBeTruthy()
    for (const question of container.querySelectorAll(
      'button[aria-expanded]',
    )) {
      expect(question.getAttribute('aria-expanded')).toBe('false')
    }
    await click('What if I cancel?')
    expect(text()).toContain(
      'Your skills keep working, with your agent at normal speed.',
    )
  })

  it('tests that Yearly shows the yearly price and what it works out to', async () => {
    await mount()
    const yearly = [...container.querySelectorAll('[role="tab"]')].find(
      (tab) => nameOf(tab) === 'Yearly2 months free',
    ) as unknown as HTMLElement
    await act(async () => {
      yearly.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    await settle()

    expect(text()).toContain('$200a year')
    expect(text()).toContain('Works out to $16.67 a month.')
  })

  it('tests that a build without billing sends the reader home', async () => {
    server.plan = async () => planOf({ available: false })
    await mount()
    expect(text()).toContain('Home page')
    expect(callsTo('getPlanPricing')).toHaveLength(0)
  })

  it('tests that a plan it can’t read says so', async () => {
    server.plan = async () => {
      throw new Error('offline')
    }
    await mount()
    expect(container.querySelector('[role="alert"]')?.textContent).toBe(
      'Couldn’t check your plan. Connect to the internet and neo checks again.',
    )
  })

  it('tests that ultrafast shows the plan, the key’s first group and the computers, never the key', async () => {
    server.plan = async () => PRO
    await mount()

    expect(text()).toContain('You’re on ultrafast')
    expect(text()).toContain('Renews Nov 27, 2026')
    expect(button('Billing')).toBeTruthy()
    expect(text()).toContain('Computers2 of 5')
    expect(text()).toContain(
      'It’s in the email we sent when you paid and starts NEO-7K2QM. Keep it private.',
    )
    expect(text()).toContain('Sam’s MacBook Air')
    expect(text()).not.toContain('You’re using it now')
    expect(text()).toContain('DESKTOP-4FQ7K2L')
    expect(text()).toContain('Last used today')
    expect(text()).toContain('This computer')
    expect(button('Rename Sam’s MacBook Air')).toBeTruthy()
    expect(button('Rename DESKTOP-4FQ7K2L')).toBeTruthy()
    expect(button('Cancel ultrafast')).toBeTruthy()
    expect(text()).toContain('What’s in ultrafast')
    expect(text()).toContain('Fewer steps. Smaller AI bills.')
    expect(text()).toContain(
      'Next on Nov 27, 2026. Stripe emails you a receipt each time.',
    )
    expect(text()).not.toContain('Get ultrafast')
    expectNoKeyAnywhere()
  })

  it('tests that Show me opens the pictures at the one chosen, and Skip closes them', async () => {
    server.plan = async () => PRO
    await mount()
    expect(container.querySelector('[role="dialog"]')).toBeNull()

    await click('Show me ultrafast skills')
    expect(dialog().getAttribute('aria-label')).toBe('ultrafast skills')
    expect(dialog().textContent).toContain('Skills run in seconds')
    expect(dialog().textContent).not.toContain('Welcome to ultrafast')

    await click('Skip', dialog())
    expect(container.querySelector('[role="dialog"]')).toBeNull()

    await click('Show me Robot checks')
    expect(dialog().textContent).toContain('Robot checks, solved for you')
    expect(button('Start using ultrafast', dialog())).toBeTruthy()
  })

  it('tests that Ending offers Keep ultrafast and keeps it', async () => {
    server.plan = async () => ({ ...PRO, status: 'ending', endsAt: PERIOD_END })
    await mount()
    expect(text()).toContain('Ends Nov 27, 2026.No more charges.')
    expect(text()).not.toContain('When am I charged?')

    await click('Keep ultrafast')

    expect(callsTo('resumePlan')).toHaveLength(1)
    expect(toasts.at(-1)?.title).toBe(
      'ultrafast stays on. It renews on Nov 27, 2026',
    )
    expect(text()).toContain('You’re on ultrafast')
    expect(events('pro_resumed')).toHaveLength(1)
  })

  it('tests that a failed payment opens Stripe to update the card', async () => {
    server.plan = async () => ({
      ...PRO,
      status: 'paymentFailed',
      graceUntil: GRACE_UNTIL,
    })
    await mount()
    expect(text()).toContain('Your last payment didn’t go through.')
    expect(text()).toContain('ultrafast stays on until Dec 4, 2026.')

    await click('Update card')

    expect(callsTo('openPlanPortal')[0]?.body).toEqual({ flow: 'updateCard' })
    expect(opened).toEqual(['https://billing.stripe.com/p/session/1'])
    expect(events('pro_billing_opened')).toHaveLength(1)
  })

  it('tests that paused ultrafast offers the card and the billing page', async () => {
    server.plan = async () => ({ ...PRO, status: 'unpaid', features: [] })
    await mount()
    expect(text()).toContain('ultrafast is paused.')
    expect(text()).toContain('Update your card to turn it back on.')

    await click('Billing')
    expect(callsTo('openPlanPortal')[0]?.body).toEqual({ flow: 'billing' })
  })

  it('tests that a plan it couldn’t refresh says so under the plan', async () => {
    server.plan = async () => ({ ...PRO, stale: true })
    await mount()
    expect(text()).toContain(
      'Couldn’t check your plan. Connect to the internet and neo checks again.',
    )
  })
})

describe('what ultrafast has saved', () => {
  it('tests that ultrafast shows the tokens saved, the totals and the latest runs', async () => {
    server.plan = async () => PRO
    server.savings = async () => SAVINGS
    await mount()

    expect(text()).toContain('973ktokens saved')
    expect(
      button(
        'Page text neo read itself that never went to your AI agent, in tokens, which is what your agent’s plan charges for. Browsing tasks from before neo counted pages add their time only.',
      ),
    ).toBeTruthy()
    expect(text()).toContain('pages read141')
    expect(text()).toContain('tasks23')
    expect(text()).toContain('neo working8 min')
    expect(text()).toContain('Latest')
    expect(text()).toContain('Search flights to Singapore')
    expect(text()).toContain('ultrafast · neo did 3 steps3 steps')
    expect(text()).toContain('Search amazon.in for Colgate')
    expect(text()).toContain(
      'ultrafast · neo did it in 7 sec5 pages, 18k tokens saved',
    )
    expect(text()).toContain('ultrafast · 15 sec')
    expect(text()).toContain('9 pages, 62k tokens saved')
    const links = [...container.querySelectorAll('a')].map((link) => [
      nameOf(link),
      link.getAttribute('href'),
    ])
    expect(links).toContainEqual(['See all', '/audit'])
    expect(links.find(([name]) => name?.startsWith('Search flights'))).toEqual([
      expect.stringContaining('Search flights to Singapore'),
      '/audit/pursue-1',
    ])
    expect(text()).not.toContain('Sample numbers')
  })

  it('tests that before the first run it starts at zero and offers Show me how', async () => {
    server.plan = async () => PRO
    await mount()

    expect(text()).toContain('0tokens saved so far')
    expect(text()).toContain('Run a task with ultrafast to start.')
    expect(text()).not.toContain('Latest')

    await click('Show me how')
    expect(dialog().getAttribute('aria-label')).toBe('ultrafast browsing')
  })

  it('tests that savings it can’t load say so and Try again asks again', async () => {
    server.plan = async () => PRO
    server.savings = async () => {
      throw new Error('offline')
    }
    await mount()

    expect(container.querySelector('[role="alert"]')?.textContent).toBe(
      'Couldn’t load what ultrafast has saved.',
    )
    expect(text()).toContain('You’re on ultrafast')
    server.savings = async () => SAVINGS
    await click('Try again')
    expect(text()).toContain('973ktokens saved')
    expect(callsTo('getPlanSavings')).toHaveLength(2)
  })

  it('tests that savings show a placeholder while they load', async () => {
    server.plan = async () => PRO
    server.savings = () => new Promise<PlanSavings>(() => undefined)
    await mount()

    const section = container.querySelector(
      '[aria-label="What ultrafast has saved"]',
    )
    expect(section?.getAttribute('aria-busy')).toBe('true')
    expect(text()).not.toContain('tokens saved')
  })

  it('tests that Free never asks for savings', async () => {
    await mount()
    expect(callsTo('getPlanSavings')).toHaveLength(0)
  })
})

describe('buying ultrafast', () => {
  it('tests that Get ultrafast opens Stripe and waits, and Cancel says nothing was paid', async () => {
    await mount()
    await click('Get ultrafast')

    expect(callsTo('startPlanCheckout')[0]?.body).toEqual({
      plan: 'ultrafast',
      interval: 'month',
    })
    expect(opened).toEqual(['https://checkout.stripe.com/c/pay/cs_test_1'])
    expect(text()).toContain('Finish in the payment tab')
    expect(text()).toContain('Pay $20 with Stripe')
    expect(events('pro_checkout_started')[0]?.properties).toEqual({
      interval: 'month',
    })

    expect(text()).toContain('Nothing is charged until you subscribe.')
    expect(text()).not.toContain('Make your agents ultrafast')

    await click('Open payment page')
    expect(opened).toHaveLength(2)

    await click('Cancel')
    expect(text()).toContain('No payment was made.')
    expect(text()).toContain('Make your agents ultrafast')
    expect(events('pro_checkout_abandoned')).toHaveLength(1)
  })

  it('tests that a finished payment welcomes the reader with three pictures, then ultrafast', async () => {
    server.checkout = async () => ({ status: 'complete', plan: PRO })
    await mount()
    await click('Get ultrafast')

    expect(dialog().getAttribute('aria-label')).toBe('Welcome to ultrafast')
    expect(dialog().textContent).toContain('Fewer steps. Smaller AI bills.')
    expect(dialog().textContent).not.toContain('licence key')
    expect(events('pro_checkout_completed')).toHaveLength(1)
    expectNoKeyAnywhere()

    await click('Next', dialog())
    expect(dialog().textContent).toContain('Skills run in seconds')
    await click('Next', dialog())
    expect(dialog().textContent).toContain('Robot checks, solved for you')

    await click('Start using ultrafast', dialog())
    expect(container.querySelector('[role="dialog"]')).toBeNull()
    expect(text()).toContain('You’re on ultrafast')
  })

  it('tests that Skip on the welcome closes it for good', async () => {
    server.checkout = async () => ({ status: 'complete', plan: PRO })
    await mount()
    await click('Get ultrafast')

    await click('Skip', dialog())
    expect(container.querySelector('[role="dialog"]')).toBeNull()
    await settle()
    expect(container.querySelector('[role="dialog"]')).toBeNull()
  })

  it('tests that a payment still being confirmed says so', async () => {
    server.checkout = async () => ({ status: 'complete', plan: null })
    await mount()
    await click('Get ultrafast')
    expect(text()).toContain('Payment received')
    expect(text()).toContain('ultrafast turns on here')
    expect(text()).not.toContain('Open payment page')
  })

  it('tests that paying again on a key whose ultrafast ended welcomes the reader back', async () => {
    server.plan = async () => planOf({ onKey: true })
    server.checkout = async () => ({ status: 'complete', plan: PRO })
    await mount()
    expect(text()).toContain('Renew and your key works again.')
    expect(text()).toContain('Turn ultrafast back on')
    expect(text()).not.toContain('Use a licence key')

    await click('Renew ultrafast')

    expect(callsTo('startPlanCheckout')[0]?.body).toEqual({
      plan: 'ultrafast',
      interval: 'month',
    })
    expect(dialog().getAttribute('aria-label')).toBe('Welcome to ultrafast')
  })

  it('tests that an expired checkout says no payment was made', async () => {
    server.checkout = async () => ({ status: 'expired', plan: null })
    await mount()
    await click('Get ultrafast')
    expect(text()).toContain('No payment was made.')
    expect(events('pro_checkout_abandoned')).toHaveLength(1)
  })

  it('tests that a computer already paying sees the server’s reason', async () => {
    server.startCheckout = async () => {
      throw apiError(409, {
        code: 'already_subscribed',
        message: 'This computer already pays for ultrafast.',
      })
    }
    await mount()
    await click('Get ultrafast')
    expect(toasts.at(-1)).toMatchObject({
      title: 'Couldn’t open the payment page',
      description: 'This computer already pays for ultrafast.',
      status: 'error',
    })
    expect(events('pro_checkout_refused')[0]?.properties).toEqual({
      reason: 'already_subscribed',
    })
    expect(opened).toEqual([])
  })

  it('tests that a changed price fetches the prices again', async () => {
    server.startCheckout = async () => {
      throw apiError(400, { code: 'unknown_plan', message: 'Unknown price.' })
    }
    await mount()
    expect(callsTo('getPlanPricing')).toHaveLength(1)
    await click('Get ultrafast')
    expect(toasts.at(-1)?.title).toBe(
      'That price changed. Check the new one and try again.',
    )
    expect(callsTo('getPlanPricing')).toHaveLength(2)
  })
})

describe('Use a licence key', () => {
  async function openDialog() {
    await mount()
    await click('Use a licence key')
    expect(events('pro_key_dialog_opened')).toHaveLength(1)
    return field('Licence key')
  }

  async function turnOn(value: string) {
    await type(field('Licence key'), value)
    await click('Turn on ultrafast', dialog())
  }

  it('tests that part of a key is caught here, without asking the server', async () => {
    await openDialog()
    expect(text()).toContain('It’s in the email we sent when you paid.')
    await turnOn('NEO-7K2QM')
    expect(text()).toContain('Paste the whole key. It starts with NEO-.')
    expect(callsTo('activatePlanKey')).toHaveLength(0)
  })

  it('tests that a wrong key says so and empties the field', async () => {
    server.activate = async () => {
      throw apiError(400, { code: 'key_invalid', message: 'No such key.' })
    }
    await openDialog()
    await turnOn(PASTED)

    expect(callsTo('activatePlanKey')[0]?.body).toEqual({ key: PASTED })
    expect(text()).toContain(
      'That key didn’t work. Copy it again from your other computer and paste it here.',
    )
    expect(field('Licence key').value).toBe('')
    expect(events('pro_key_activation_failed')[0]?.properties).toEqual({
      reason: 'key_invalid',
    })
    expectNoKeyAnywhere()
  })

  it('tests that too many tries asks the reader to wait', async () => {
    server.activate = async () => {
      throw apiError(429, {
        code: 'too_many_tries',
        message: 'Slow down.',
        retryAfterSeconds: 600,
      })
    }
    await openDialog()
    await turnOn(PASTED)
    expect(text()).toContain(
      'Too many tries from this computer. Wait a few minutes, then try again.',
    )
  })

  it('tests that a computer already paying sees the server’s reason', async () => {
    server.activate = async () => {
      throw apiError(409, {
        code: 'already_subscribed',
        message: 'This computer already pays for ultrafast. Contact support.',
      })
    }
    await openDialog()
    await turnOn(PASTED)
    expect(text()).toContain(
      'This computer already pays for ultrafast. Contact support.',
    )
  })

  it('tests that the help for a lost key points to the email', async () => {
    await openDialog()
    await click('Can’t find your key?')
    expect(text()).toContain('Search your email for “ultrafast licence key”.')
  })

  it('tests that a full key lists its computers and Remove makes room', async () => {
    const five = Array.from({ length: 5 }, (_, index) =>
      computer({
        installId: `computer-${index}`,
        label: `Computer ${index}`,
        thisComputer: false,
      }),
    )
    server.activate = async (body) => {
      if (!body.replace) {
        throw apiError(409, {
          code: 'key_full',
          message: 'This key is on 5 computers.',
          computers: five,
        })
      }
      return { plan: PRO, key: { ...KEY, computers: [THIS, ...five.slice(1)] } }
    }
    await openDialog()
    await turnOn(PASTED)

    expect(text()).toContain('This key is on 5 computers')
    expect(text()).toContain(
      'Remove one to make room for this computer. The one you remove switches to Free the next time it checks in.',
    )
    expect(text()).toContain('Computer 4')

    const rows = dialog().querySelectorAll('li')
    await click('Remove', rows[0] as unknown as Element)

    expect(callsTo('activatePlanKey')[1]?.body).toEqual({
      key: PASTED,
      replace: 'computer-0',
    })
    expect(text()).toContain('ultrafast is on for this computer')
    expect(text()).toContain(
      'It renews on Nov 27, 2026. Your key is now on 5 of 5 computers.',
    )
    expectNoKeyAnywhere()
  })

  it('tests that a key whose ultrafast ended offers Get ultrafast again on that key', async () => {
    server.activate = async () => ({ plan: planOf({ onKey: true }), key: KEY })
    await openDialog()
    await turnOn(PASTED)

    expect(text()).toContain('This key’s ultrafast plan has ended')
    expect(events('pro_key_activated')[0]?.properties).toEqual({
      plan_ended: true,
    })
    await click('Get ultrafast again')

    expect(callsTo('startPlanCheckout')).toHaveLength(1)
    expect(text()).toContain('Finish in the payment tab')
  })

  it('tests that a working key turns on ultrafast here', async () => {
    await openDialog()
    await turnOn(PASTED)

    expect(text()).toContain('ultrafast is on for this computer')
    expect(text()).toContain('Your key is now on 2 of 5 computers.')
    await click('Done')
    expect(text()).toContain('You’re on ultrafast')
    expect(text()).toContain('and starts NEO-7K2QM.')
    expectNoKeyAnywhere()
  })
})

describe('the computers on the key', () => {
  async function mountPro() {
    server.plan = async () => PRO
    await mount()
  }

  it('tests that Rename saves the new name', async () => {
    await mountPro()
    await click('Rename DESKTOP-4FQ7K2L')
    const name = field('Name for this computer')
    expect(name.value).toBe('DESKTOP-4FQ7K2L')

    await type(name, '  Studio PC ')
    await click('Save')

    expect(callsTo('renamePlanComputer')[0]?.body).toEqual({
      installId: 'other-computer',
      name: 'Studio PC',
    })
    expect(toasts.at(-1)?.title).toBe('Renamed to Studio PC')
    expect(button('Rename Studio PC')).toBeTruthy()
    expect(events('pro_computer_renamed')[0]?.properties).toBeUndefined()
  })

  it('tests that Escape and Cancel leave the name as it was', async () => {
    await mountPro()
    await click('Rename DESKTOP-4FQ7K2L')
    await type(field('Name for this computer'), 'Something else')
    await pressKey(field('Name for this computer'), 'Escape')
    expect(button('Rename DESKTOP-4FQ7K2L')).toBeTruthy()

    await click('Rename DESKTOP-4FQ7K2L')
    await click('Cancel')
    expect(button('Rename DESKTOP-4FQ7K2L')).toBeTruthy()
    expect(callsTo('renamePlanComputer')).toHaveLength(0)
  })

  it('tests that an empty name asks for one', async () => {
    await mountPro()
    await click('Rename DESKTOP-4FQ7K2L')
    await type(field('Name for this computer'), '   ')
    await click('Save')
    expect(text()).toContain('Give this computer a name.')
    expect(callsTo('renamePlanComputer')).toHaveLength(0)
  })

  it('tests that a failed rename keeps the typed name', async () => {
    server.rename = async () => {
      throw apiError(503, {
        code: 'billing_unreachable',
        message: 'neo couldn’t reach the billing service.',
      })
    }
    await mountPro()
    await click('Rename DESKTOP-4FQ7K2L')
    await type(field('Name for this computer'), 'Studio PC')
    await click('Save')

    expect(toasts.at(-1)).toMatchObject({
      title: 'Couldn’t rename this computer',
      description: 'neo couldn’t reach the billing service.',
    })
    expect(field('Name for this computer').value).toBe('Studio PC')
  })

  it('tests that Remove asks first, then takes the computer off', async () => {
    await mountPro()
    await click('Remove')

    expect(dialog().textContent).toContain('Remove DESKTOP-4FQ7K2L?')
    await click('Remove', dialog())

    expect(callsTo('removePlanComputer')[0]?.body).toBe('other-computer')
    expect(toasts.at(-1)?.title).toBe('Removed DESKTOP-4FQ7K2L')
    expect(text()).not.toContain('DESKTOP-4FQ7K2L')
    expect(text()).toContain('Computers1 of 5')
  })

  it('tests that Reset says the new key was emailed, and never shows it', async () => {
    await mountPro()
    await click('Reset key')
    expect(dialog().textContent).toContain('We’ll email you the new key.')

    await click('Reset key', dialog())

    expect(callsTo('resetPlanKey')).toHaveLength(1)
    expect(dialog().getAttribute('aria-label')).toBe(
      'We’ve emailed your new key',
    )
    expect(dialog().textContent).toContain(
      'Your old key no longer works. Paste the new one on your other computers to turn ultrafast back on there.',
    )
    expect(text()).toContain('and starts NEO-Q3VTM.')
    expectNoKeyAnywhere()
  })
})

describe('cancelling ultrafast', () => {
  const FEEDBACK = 'Too pricey for me right now'

  async function openCancel() {
    server.plan = async () => PRO
    await mount()
    await click('Cancel ultrafast')
    expect(dialog().textContent).toContain(
      'You keep ultrafast until Nov 27, 2026. After that, neo goes back to Free and you aren’t charged again.',
    )
  }

  function feedbackBox(): HTMLTextAreaElement {
    return container.querySelector('textarea') as unknown as HTMLTextAreaElement
  }

  it('tests that cancelling with feedback sends it and thanks the reader', async () => {
    await openCancel()
    feedbackBox().value = FEEDBACK
    await click('Cancel ultrafast', dialog())

    expect(callsTo('cancelPlan')[0]?.body).toEqual({ feedback: FEEDBACK })
    expect(toasts.at(-1)?.title).toBe(
      'Thanks for telling us. ultrafast ends on Nov 27, 2026',
    )
    expect(text()).toContain('Ends Nov 27, 2026.')
    expect(events('pro_cancel_confirmed')[0]?.properties).toEqual({
      gave_feedback: true,
    })
    expect(JSON.stringify(captured)).not.toContain(FEEDBACK)
  })

  it('tests that cancelling without feedback sends none', async () => {
    await openCancel()
    await click('Cancel ultrafast', dialog())
    expect(callsTo('cancelPlan')[0]?.body).toEqual({})
    expect(toasts.at(-1)?.title).toBe('ultrafast will end on Nov 27, 2026')
  })

  it('tests that a failed cancel keeps the feedback and changes nothing', async () => {
    server.cancel = async () => {
      throw new Error('offline')
    }
    await openCancel()
    feedbackBox().value = FEEDBACK
    await click('Cancel ultrafast', dialog())

    expect(toasts.at(-1)).toMatchObject({
      title: 'Couldn’t cancel ultrafast. Nothing changed.',
      status: 'error',
    })
    expect(feedbackBox().value).toBe(FEEDBACK)
    expect(text()).toContain('You’re on ultrafast')
  })

  it('tests that Keep ultrafast closes the dialog without cancelling', async () => {
    await openCancel()
    await click('Keep ultrafast', dialog())
    expect(container.querySelector('[role="dialog"]')).toBeNull()
    expect(callsTo('cancelPlan')).toHaveLength(0)
  })
})
