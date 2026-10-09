import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'

mock.module('@/modules/analytics/telemetry.hooks', () => ({
  useTelemetryState: () => ({ data: undefined, isError: false }),
}))

mock.module('@/modules/analytics/events', () => ({
  AnalyticsEvent: { UltrafastBannerClicked: 'ultrafast_banner_clicked' },
  track: () => {},
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

const { UltrafastBanner } = await import('./UltrafastBanner')
const { rememberJoined } = await import('@/screens/ultrafast/ultrafast-price')

let root: Root
let container: HTMLElement

beforeEach(async () => {
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

async function render() {
  await act(async () => {
    root.render(
      <MemoryRouter>
        <UltrafastBanner />
      </MemoryRouter>,
    )
  })
}

describe('UltrafastBanner', () => {
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
