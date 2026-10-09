import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, createElement } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'

interface Tracked {
  event: string
  properties?: Record<string, unknown>
}

const state: {
  telemetry: { data?: { distinctId: string }; isError: boolean }
  capturing: boolean
  tracked: Tracked[]
} = {
  telemetry: { isError: false },
  capturing: true,
  tracked: [],
}

mock.module('@/modules/analytics/telemetry.hooks', () => ({
  useTelemetryState: () => state.telemetry,
}))

const captureStateListeners = new Set<() => void>()

mock.module('@/modules/analytics/posthog', () => ({
  isCapturing: () => state.capturing,
  subscribeToCaptureState: (listener: () => void) => {
    captureStateListeners.add(listener)
    return () => captureStateListeners.delete(listener)
  },
}))

mock.module('@/modules/analytics/events', () => ({
  AnalyticsEvent: {
    UltrafastWaitlistViewed: 'ultrafast_waitlist_viewed',
    UltrafastWaitlistJoined: 'ultrafast_waitlist_joined',
  },
  track: (event: string, properties?: Record<string, unknown>) => {
    state.tracked.push({ event, properties })
  },
}))

mock.module('@/components/ui/button', () => ({
  Button: (props: ComponentProps<'button'>) =>
    createElement('button', { type: 'button', ...props }),
}))

mock.module('@/components/ui/input', () => ({
  Input: (props: ComponentProps<'input'>) => createElement('input', props),
}))

const storage: Record<string, string> = {}
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => storage[key] ?? null,
    setItem: (key: string, value: string) => {
      storage[key] = value
    },
    removeItem: (key: string) => {
      delete storage[key]
    },
  },
})

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

const { UltrafastWaitlist, UltrafastWaitlistView, normalizeEmail } =
  await import('./UltrafastWaitlist')
const { priceForId } = await import('./ultrafast-price')

const JOINED_KEY = 'ultrafastWaitlistJoined:v1'

/** Two ids that land on different prices, found rather than hard-coded. */
function idsWithDifferentPrices(): [string, string] {
  const first = 'id-0'
  for (let i = 1; ; i++) {
    const other = `id-${i}`
    if (priceForId(other) !== priceForId(first)) return [first, other]
  }
}

let root: Root
let container: HTMLElement

beforeEach(async () => {
  state.telemetry = { isError: false }
  state.capturing = true
  state.tracked = []
  captureStateListeners.clear()
  for (const key of Object.keys(storage)) delete storage[key]

  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  // react-dom probes `'oninput' in document` once at load; linkedom lacks the
  // property, which would make React ignore input events and miss typing.
  Object.defineProperty(dom.document, 'oninput', {
    configurable: true,
    writable: true,
    value: null,
  })
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
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

async function render() {
  await act(async () => {
    root.render(<UltrafastWaitlist />)
  })
}

async function setCapturing(capturing: boolean) {
  await act(async () => {
    state.capturing = capturing
    for (const listener of captureStateListeners) listener()
  })
}

function input(): HTMLInputElement {
  const match = container.querySelector('input[type="email"]')
  if (!match) throw new Error('no email input')
  return match as unknown as HTMLInputElement
}

async function submit(email: string) {
  await act(async () => {
    const field = input()
    // React tracks the last value it saw; going through the prototype setter
    // makes the change visible to its onChange.
    const setter = Object.getOwnPropertyDescriptor(
      Object.getPrototypeOf(field),
      'value',
    )?.set
    setter?.call(field, email)
    field.dispatchEvent(new window.Event('input', { bubbles: true }))
  })
  await act(async () => {
    const form = container.querySelector('form')
    if (!form) throw new Error('no form')
    form.dispatchEvent(
      new window.Event('submit', { bubbles: true, cancelable: true }),
    )
  })
}

const views = () =>
  state.tracked.filter((t) => t.event === 'ultrafast_waitlist_viewed')
const joins = () =>
  state.tracked.filter((t) => t.event === 'ultrafast_waitlist_joined')

describe('UltrafastWaitlist', () => {
  it('shows a loading state until the price is known', async () => {
    await render()

    expect(container.textContent).toContain('Loading')
    expect(container.textContent).not.toContain('/month')
    expect(state.tracked).toEqual([])
  })

  it('explains an unreachable server, then recovers into the real price', async () => {
    state.telemetry = { isError: true }
    await render()
    expect(container.textContent).toContain("Can't reach neo's local service")
    expect(state.tracked).toEqual([])

    state.telemetry = { data: { distinctId: 'id-0' }, isError: false }
    await render()
    const price = priceForId('id-0')
    expect(container.textContent).toContain(`$${price}`)
    expect(views()).toEqual([
      {
        event: 'ultrafast_waitlist_viewed',
        properties: { price_usd: price, already_joined: false },
      },
    ])
  })

  it('counts the view once, at the price shown', async () => {
    state.telemetry = { data: { distinctId: 'id-0' }, isError: false }
    await render()
    await render()

    expect(views()).toHaveLength(1)
    expect(views()[0]?.properties?.price_usd).toBe(priceForId('id-0'))
  })

  it('keeps the shown price when the analytics id changes mid-visit', async () => {
    const [first, second] = idsWithDifferentPrices()
    state.telemetry = { data: { distinctId: first }, isError: false }
    await render()

    state.telemetry = { data: { distinctId: second }, isError: false }
    await render()

    expect(container.textContent).toContain(`$${priceForId(first)}`)
    expect(container.textContent).not.toContain(`$${priceForId(second)}`)
  })

  it('waits for capture before counting the view and allowing signup', async () => {
    state.telemetry = { data: { distinctId: 'id-0' }, isError: false }
    state.capturing = false
    await render()

    expect(views()).toEqual([])
    expect(input().disabled).toBe(true)
    expect(container.textContent).toContain('usage analytics, which is off')

    await setCapturing(true)
    expect(views()).toHaveLength(1)
    expect(input().disabled).toBe(false)
  })

  it('sends the email with the shown price and confirms the signup', async () => {
    state.telemetry = { data: { distinctId: 'id-0' }, isError: false }
    await render()
    await submit('  Ada@Example.com ')

    expect(joins()).toEqual([
      {
        event: 'ultrafast_waitlist_joined',
        properties: { price_usd: priceForId('id-0'), email: 'ada@example.com' },
      },
    ])
    expect(storage[JOINED_KEY]).toBe('true')
    expect(container.textContent).toContain('on the list')
    expect(container.querySelector('form')).toBeNull()
  })

  it('rejects an invalid email without sending anything', async () => {
    state.telemetry = { data: { distinctId: 'id-0' }, isError: false }
    await render()
    await submit('ada@example')

    expect(joins()).toEqual([])
    expect(container.textContent).toContain('Enter a valid email address.')
    expect(storage[JOINED_KEY]).toBeUndefined()
  })

  it('does not remember a join when capture stopped before submit', async () => {
    state.telemetry = { data: { distinctId: 'id-0' }, isError: false }
    await render()
    // Capture stops without the page having re-rendered yet.
    state.capturing = false
    await submit('ada@example.com')

    expect(joins()).toEqual([])
    expect(storage[JOINED_KEY]).toBeUndefined()
    expect(container.querySelector('form')).not.toBeNull()
  })

  it('picks up a join made in another tab', async () => {
    state.telemetry = { data: { distinctId: 'id-0' }, isError: false }
    await render()
    expect(container.querySelector('form')).not.toBeNull()

    storage[JOINED_KEY] = 'true'
    await act(async () => {
      const event = new window.Event('storage')
      Object.defineProperty(event, 'key', { value: JOINED_KEY })
      window.dispatchEvent(event)
    })

    expect(container.textContent).toContain('on the list')
    expect(container.querySelector('form')).toBeNull()
  })
})

describe('UltrafastWaitlistView', () => {
  it.each([9, 19] as const)(
    'renders the $%i price, form, and launch note',
    (price) => {
      const html = renderToStaticMarkup(
        createElement(UltrafastWaitlistView, {
          price,
          joined: false,
          canJoin: true,
          onJoin: () => {},
        }),
      )
      expect(html).toContain(`$${price}`)
      expect(html).toContain('/month')
      expect(html).toContain('Request early access</button>')
      expect(html).toContain('No payment today.')
    },
  )
})

describe('normalizeEmail', () => {
  it('trims and lowercases a valid email', () => {
    expect(normalizeEmail('  Ada@Example.COM ')).toBe('ada@example.com')
  })

  it('rejects values that are not emails', () => {
    expect(normalizeEmail('')).toBeNull()
    expect(normalizeEmail('ada')).toBeNull()
    expect(normalizeEmail('ada@example')).toBeNull()
    expect(normalizeEmail('a da@example.com')).toBeNull()
  })
})
