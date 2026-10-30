import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { PlanPricing } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'
import * as api from '@/modules/api/client'

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

const server: { pricing: () => Promise<PlanPricing> } = {
  pricing: async () => PRICING,
}

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({ getPlanPricing: () => server.pricing() }),
}))

mock.module('@/modules/analytics/telemetry.hooks', () => ({
  useTelemetryState: () => ({ data: undefined, isError: false }),
}))

const tracked: string[] = []
mock.module('@/modules/analytics/events', () => ({
  AnalyticsEvent: { UltrafastBannerClicked: 'ultrafast_banner_clicked' },
  track: (event: string) => tracked.push(event),
}))

const storage: Record<string, string> = {}
let storageUnavailable = false
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => {
      if (storageUnavailable) throw new Error('Storage unavailable')
      return storage[key] ?? null
    },
    setItem: (key: string, value: string) => {
      if (storageUnavailable) throw new Error('Storage unavailable')
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

const { ProBanner } = await import('./ProBanner')

let root: Root
let container: HTMLElement
let client: QueryClient

beforeEach(async () => {
  tracked.length = 0
  storageUnavailable = false
  server.pricing = async () => PRICING
  for (const key of Object.keys(storage)) delete storage[key]
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
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
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  client.clear()
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

async function render(key = 'initial') {
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <ProBanner key={key} />
        </MemoryRouter>
      </QueryClientProvider>,
    )
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

function closeButton() {
  return container.querySelector('button[aria-label="Hide this"]')
}

describe('ProBanner', () => {
  it('tests that it names the monthly price and links to the neo Pro page', async () => {
    await render()

    expect(container.textContent).toContain('neo Pro')
    expect(container.textContent).toContain(
      'Faster tasks, a year of history and more, for $20 a month.',
    )
    const link = container.querySelector('a')
    expect(link?.textContent).toContain('See Pro')
    expect(link?.getAttribute('href')).toBe('/pro')
  })

  it('tests that it stays away while the prices can’t be read', async () => {
    server.pricing = async () => {
      throw new Error('Gateway down')
    }
    await render()
    expect(container.innerHTML).toBe('')
  })

  it('tests that dismissal stays hidden on a new mount without opening the page', async () => {
    await render()
    const close = closeButton()
    expect(close).not.toBeNull()
    expect(close?.closest('a')).toBeNull()

    await act(async () => {
      close?.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(container.innerHTML).toBe('')
    expect(tracked).toEqual([])

    await render('new-tab')
    expect(container.innerHTML).toBe('')
  })

  it('tests that dismissal in another tab hides an already visible banner', async () => {
    await render()
    expect(container.querySelector('a')).not.toBeNull()
    storage['proBannerDismissed:v1'] = 'true'

    await act(async () => {
      const event = new window.Event('storage')
      Object.defineProperty(event, 'key', { value: 'proBannerDismissed:v1' })
      window.dispatchEvent(event)
    })
    expect(container.innerHTML).toBe('')
  })

  it('tests that dismissal still hides the current banner when storage is unavailable', async () => {
    storageUnavailable = true
    await render()
    await act(async () => {
      closeButton()?.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(container.innerHTML).toBe('')
  })
})
