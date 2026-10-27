import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { useFaviconSources } from '@/modules/favicon/favicon.hooks'
import { Favicon } from './Favicon'

const SOURCES = [
  'chrome-extension://neo/_favicon/?pageUrl=https%3A%2F%2Fx.com&size=32',
  'https://icons.duckduckgo.com/ip2/x.com.ico',
  'https://www.google.com/s2/favicons?sz=32&domain_url=x.com',
]
const names = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'IS_REACT_ACT_ENVIRONMENT',
]
const descriptors = new Map(
  names.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)
let root: Root
let container: HTMLElement

beforeEach(() => {
  const dom = parseHTML('<html><body><div id="root"></div></body></html>')
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    IS_REACT_ACT_ENVIRONMENT: true,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  container = dom.document.getElementById('root') as unknown as HTMLElement
  root = createRoot(container)
})

afterEach(() => {
  act(() => root.unmount())
  for (const [name, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else delete (globalThis as Record<string, unknown>)[name]
  }
})

function show(sources: string[]) {
  const client = new QueryClient()
  client.setQueryData(
    useFaviconSources.getKey({ site: 'x.com', size: 32 }),
    sources,
  )
  act(() => {
    root.render(
      <QueryClientProvider client={client}>
        <Favicon site="x.com" fallback={<span>globe</span>} />
      </QueryClientProvider>,
    )
  })
}

function failToLoad() {
  const image = container.querySelector('img')
  act(() => {
    image?.dispatchEvent(new Event('error'))
  })
}

describe('Favicon', () => {
  it('tries each source in turn, then shows the fallback', () => {
    show(SOURCES)
    expect(container.querySelector('img')?.getAttribute('src')).toBe(SOURCES[0])
    failToLoad()
    expect(container.querySelector('img')?.getAttribute('src')).toBe(SOURCES[1])
    failToLoad()
    expect(container.querySelector('img')?.getAttribute('src')).toBe(SOURCES[2])
    failToLoad()
    expect(container.querySelector('img')).toBeNull()
    expect(container.textContent).toBe('globe')
  })

  it('shows the fallback when a site has no sources', () => {
    show([])
    expect(container.textContent).toBe('globe')
  })
})
