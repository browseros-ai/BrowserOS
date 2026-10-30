/**
 * The neo Pro page against a stubbed claw-server. The real react-query hooks
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
  plan: 'pro',
  planName: 'neo Pro',
  features: ['claw.ultrafast'],
  status: 'active',
  periodEnd: PERIOD_END,
  onKey: true,
})

const PRICING: PlanPricing = {
  plans: [
    {
      plan: 'pro',
      name: 'neo Pro',
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

function apiError(status: number, body: Record<string, unknown>) {
  return new ApiResponseError(new Response(null, { status }), body)
}

interface Server {
  plan: () => Promise<Plan>
  pricing: () => Promise<PlanPricing>
  key: () => Promise<PlanKey>
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

describe('the neo Pro page', () => {
  it('tests that Free shows the monthly price, the way in with a key, and the questions', async () => {
    await mount()

    expect(text()).toContain(
      'More from your AI agents, for $20 a month, or less if you pay yearly. Cancel whenever you like.',
    )
    expect(text()).toContain('Ultrafast mode included')
    expect(text()).toContain('$20a month')
    expect(text()).toContain('Renews by itself each month')
    expect(text()).toContain('2 months free')
    expect(button('Get Pro')).toBeTruthy()
    expect(button('Use a licence key')).toBeTruthy()
    expect(text()).toContain('Questions people ask')
    expect(text()).toContain('Where’s my licence key?')
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
    expect(text()).toContain('Renews by itself each year')
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

  it('tests that Pro shows the plan, the key’s first group and the computers, never the key', async () => {
    server.plan = async () => PRO
    await mount()

    expect(text()).toContain(
      'Thanks for supporting BrowserOS. Here’s what Pro adds to neo.',
    )
    expect(text()).toContain('You’re on Pro')
    expect(text()).toContain('Next paymentNov 27, 2026')
    expect(text()).toContain('Computers2 of 5')
    expect(text()).toContain('It’s in the email we sent when you paid.')
    expect(text()).toContain('It starts NEO-7K2QM.')
    expect(text()).toContain(
      'Can’t find the email? Reset your key and we’ll email you a new one.',
    )
    expect(text()).toContain('Sam’s MacBook Air')
    expect(text()).toContain(
      'Added Oct 27, 2026, 9:41 AM · You’re using it now',
    )
    expect(text()).toContain('DESKTOP-4FQ7K2L')
    expect(text()).toContain('Last used today')
    expect(text()).toContain('This computer')
    expect(button('Rename Sam’s MacBook Air')).toBeTruthy()
    expect(button('Rename DESKTOP-4FQ7K2L')).toBeTruthy()
    expect(button('Cancel Pro')).toBeTruthy()
    expectNoKeyAnywhere()
  })

  it('tests that Ending offers Keep Pro and keeps it', async () => {
    server.plan = async () => ({ ...PRO, status: 'ending', endsAt: PERIOD_END })
    await mount()
    expect(text()).toContain('Pro ends on Nov 27, 2026')
    expect(text()).toContain('You won’t be charged again.')

    await click('Keep Pro')

    expect(callsTo('resumePlan')).toHaveLength(1)
    expect(toasts.at(-1)?.title).toBe('Pro stays on. It renews on Nov 27, 2026')
    expect(text()).toContain('You’re on Pro')
    expect(events('pro_resumed')).toHaveLength(1)
  })

  it('tests that a failed payment opens Stripe to update the card', async () => {
    server.plan = async () => ({
      ...PRO,
      status: 'paymentFailed',
      graceUntil: GRACE_UNTIL,
    })
    await mount()
    expect(text()).toContain('Your card needs a look')
    expect(text()).toContain('Pro stays on until Dec 4, 2026.')

    await click('Update card')

    expect(callsTo('openPlanPortal')[0]?.body).toEqual({ flow: 'updateCard' })
    expect(opened).toEqual(['https://billing.stripe.com/p/session/1'])
    expect(events('pro_billing_opened')).toHaveLength(1)
  })

  it('tests that paused Pro offers the card and the billing page', async () => {
    server.plan = async () => ({ ...PRO, status: 'unpaid', features: [] })
    await mount()
    expect(text()).toContain('Pro is paused')
    expect(text()).toContain('Your last few payments didn’t go through.')

    await click('Billing and receipts')
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

describe('buying Pro', () => {
  it('tests that Get Pro opens Stripe and waits, and Cancel says nothing was paid', async () => {
    await mount()
    await click('Get Pro')

    expect(callsTo('startPlanCheckout')[0]?.body).toEqual({
      plan: 'pro',
      interval: 'month',
    })
    expect(opened).toEqual(['https://checkout.stripe.com/c/pay/cs_test_1'])
    expect(text()).toContain('Finish in the payment tab')
    expect(text()).toContain('Pay $20 with Stripe')
    expect(events('pro_checkout_started')[0]?.properties).toEqual({
      interval: 'month',
    })

    await click('Open the payment page again')
    expect(opened).toHaveLength(2)

    await click('Cancel')
    expect(text()).toContain(
      'No payment was made. You can try again whenever you like.',
    )
    expect(events('pro_checkout_abandoned')).toHaveLength(1)
  })

  it('tests that a finished payment shows Welcome with the emailed key line, and Pro', async () => {
    server.checkout = async () => ({ status: 'complete', plan: PRO })
    await mount()
    await click('Get Pro')

    const welcome = dialog()
    expect(welcome.getAttribute('aria-label')).toBe('You’re on neo Pro')
    expect(welcome.textContent).toContain(
      'Thank you! We’ve emailed your licence key, and Stripe is emailing your receipt.',
    )
    expect(welcome.textContent).toContain(
      'Use the key in that email to turn on Pro on your other computers.',
    )
    expect(welcome.textContent).toContain(
      'Renews on Nov 27, 2026. Billing, receipts and cancelling live on the neo Pro page.',
    )
    expect(events('pro_checkout_completed')).toHaveLength(1)
    expectNoKeyAnywhere()

    await click('Start using Pro')
    expect(container.querySelector('[role="dialog"]')).toBeNull()
    expect(text()).toContain('You’re on Pro')
  })

  it('tests that a payment still being confirmed says so', async () => {
    server.checkout = async () => ({ status: 'complete', plan: null })
    await mount()
    await click('Get Pro')
    expect(text()).toContain('Payment received')
    expect(text()).toContain('One moment while neo turns on Pro.')
  })

  it('tests that paying again on a key whose Pro ended keeps the same key', async () => {
    server.plan = async () => planOf({ onKey: true })
    server.checkout = async () => ({ status: 'complete', plan: PRO })
    await mount()
    expect(text()).toContain(
      'This computer’s licence key has no Pro right now. Get Pro to start it again.',
    )

    await click('Get Pro')

    const welcome = dialog().textContent ?? ''
    expect(welcome).toContain('Thank you! Stripe is emailing your receipt.')
    expect(welcome).toContain(
      'Your key stays the same. Pro is back on every computer that uses it.',
    )
    expect(welcome).not.toContain('We’ve emailed your licence key')
  })

  it('tests that an expired checkout says no payment was made', async () => {
    server.checkout = async () => ({ status: 'expired', plan: null })
    await mount()
    await click('Get Pro')
    expect(text()).toContain('No payment was made.')
    expect(events('pro_checkout_abandoned')).toHaveLength(1)
  })

  it('tests that a computer already paying sees the server’s reason', async () => {
    server.startCheckout = async () => {
      throw apiError(409, {
        code: 'already_subscribed',
        message: 'This computer already pays for Pro.',
      })
    }
    await mount()
    await click('Get Pro')
    expect(toasts.at(-1)).toMatchObject({
      title: 'Couldn’t open the payment page',
      description: 'This computer already pays for Pro.',
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
    await click('Get Pro')
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
    await click('Turn on Pro', dialog())
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
        message: 'This computer already pays for Pro. Contact support.',
      })
    }
    await openDialog()
    await turnOn(PASTED)
    expect(text()).toContain(
      'This computer already pays for Pro. Contact support.',
    )
  })

  it('tests that the help for a lost key points to the email', async () => {
    await openDialog()
    await click('Can’t find your key?')
    expect(text()).toContain('Search your email for “neo Pro licence key”.')
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
    expect(text()).toContain('Pro is on for this computer')
    expect(text()).toContain(
      'It renews on Nov 27, 2026. Your key is now on 5 of 5 computers.',
    )
    expectNoKeyAnywhere()
  })

  it('tests that a key whose Pro ended offers Get Pro again on that key', async () => {
    server.activate = async () => ({ plan: planOf({ onKey: true }), key: KEY })
    await openDialog()
    await turnOn(PASTED)

    expect(text()).toContain('This key’s Pro has ended')
    expect(events('pro_key_activated')[0]?.properties).toEqual({
      plan_ended: true,
    })
    await click('Get Pro again')

    expect(callsTo('startPlanCheckout')).toHaveLength(1)
    expect(text()).toContain('Finish in the payment tab')
  })

  it('tests that a working key turns on Pro here', async () => {
    await openDialog()
    await turnOn(PASTED)

    expect(text()).toContain('Pro is on for this computer')
    expect(text()).toContain('Your key is now on 2 of 5 computers.')
    await click('Done')
    expect(text()).toContain('You’re on Pro')
    expect(text()).toContain('It starts NEO-7K2QM.')
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
      'Your old key no longer works. Paste the new one on your other computers to turn Pro back on there.',
    )
    expect(text()).toContain('It starts NEO-Q3VTM.')
    expectNoKeyAnywhere()
  })
})

describe('cancelling Pro', () => {
  const FEEDBACK = 'Too pricey for me right now'

  async function openCancel() {
    server.plan = async () => PRO
    await mount()
    await click('Cancel Pro')
    expect(dialog().textContent).toContain(
      'You keep Pro until Nov 27, 2026. After that, neo goes back to Free and you aren’t charged again.',
    )
  }

  function feedbackBox(): HTMLTextAreaElement {
    return container.querySelector('textarea') as unknown as HTMLTextAreaElement
  }

  it('tests that cancelling with feedback sends it and thanks the reader', async () => {
    await openCancel()
    feedbackBox().value = FEEDBACK
    await click('Cancel Pro', dialog())

    expect(callsTo('cancelPlan')[0]?.body).toEqual({ feedback: FEEDBACK })
    expect(toasts.at(-1)?.title).toBe(
      'Thanks for telling us. Pro ends on Nov 27, 2026',
    )
    expect(text()).toContain('Pro ends on Nov 27, 2026')
    expect(events('pro_cancel_confirmed')[0]?.properties).toEqual({
      gave_feedback: true,
    })
    expect(JSON.stringify(captured)).not.toContain(FEEDBACK)
  })

  it('tests that cancelling without feedback sends none', async () => {
    await openCancel()
    await click('Cancel Pro', dialog())
    expect(callsTo('cancelPlan')[0]?.body).toEqual({})
    expect(toasts.at(-1)?.title).toBe('Pro will end on Nov 27, 2026')
  })

  it('tests that a failed cancel keeps the feedback and changes nothing', async () => {
    server.cancel = async () => {
      throw new Error('offline')
    }
    await openCancel()
    feedbackBox().value = FEEDBACK
    await click('Cancel Pro', dialog())

    expect(toasts.at(-1)).toMatchObject({
      title: 'Couldn’t cancel Pro. Nothing changed.',
      status: 'error',
    })
    expect(feedbackBox().value).toBe(FEEDBACK)
    expect(text()).toContain('You’re on Pro')
  })

  it('tests that Keep Pro closes the dialog without cancelling', async () => {
    await openCancel()
    await click('Keep Pro', dialog())
    expect(container.querySelector('[role="dialog"]')).toBeNull()
    expect(callsTo('cancelPlan')).toHaveLength(0)
  })
})
