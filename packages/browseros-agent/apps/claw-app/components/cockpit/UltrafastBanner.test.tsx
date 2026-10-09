import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'

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

const { UltrafastBanner } = await import('./UltrafastBanner')
const { rememberJoined } = await import('@/screens/ultrafast/ultrafast-price')

let root: Root
let container: HTMLElement

beforeEach(async () => {
  tracked.length = 0
  storageUnavailable = false
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

async function render(key = 'initial') {
  await act(async () => {
    root.render(
      <MemoryRouter>
        <UltrafastBanner key={key} />
      </MemoryRouter>,
    )
  })
}

describe('UltrafastBanner', () => {
  it('tests that dismissal stays hidden on a new mount without joining or opening the waitlist', async () => {
    await render()
    const close = container.querySelector(
      'button[aria-label="Dismiss this banner"]',
    )
    expect(close).not.toBeNull()
    expect(close?.closest('a')).toBeNull()
    expect(close?.getAttribute('title')).toBe('Dismiss this banner')

    await act(async () => {
      close?.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(container.innerHTML).toBe('')
    expect(tracked).toEqual([])
    expect(storage['ultrafastWaitlistJoined:v1']).toBeUndefined()

    await render('new-tab')
    expect(container.innerHTML).toBe('')
  })

  it('tests that dismissal in another tab hides an already visible banner', async () => {
    await render()
    expect(container.querySelector('a')).not.toBeNull()
    storage['ultrafastBannerDismissed:v1'] = 'true'

    await act(async () => {
      const event = new window.Event('storage')
      Object.defineProperty(event, 'key', {
        value: 'ultrafastBannerDismissed:v1',
      })
      window.dispatchEvent(event)
    })
    expect(container.innerHTML).toBe('')
  })

  it('tests that dismissal still hides the current banner when storage is unavailable', async () => {
    storageUnavailable = true
    await render()
    const close = container.querySelector(
      'button[aria-label="Dismiss this banner"]',
    )
    expect(close).not.toBeNull()
    await act(async () => {
      close?.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(container.innerHTML).toBe('')
  })

  it('links to the waitlist', async () => {
    await render()

    expect(container.textContent).toContain('Ultrafast mode')
    expect(container.textContent).toContain('Request early access')
    expect(container.querySelector('a')?.getAttribute('href')).toBe(
      '/ultrafast',
    )
  })

  it('goes away as soon as the reader joins', async () => {
    await render()

    await act(async () => rememberJoined())

    expect(container.innerHTML).toBe('')
  })
})
