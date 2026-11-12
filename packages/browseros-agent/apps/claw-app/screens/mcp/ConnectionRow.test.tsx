import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import type { ToastInput } from '@/components/motion/animated-toast-stack'
import type { ConnectionState } from '@/modules/api/connections.hooks'
import { connectToastHost } from '@/modules/toast/toast'

const { ConnectionRow } = await import('./ConnectionRow')
const { McpAddressCard } = await import('./McpAddressCard')

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'Node',
    'Event',
    'matchMedia',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

let root: Root
let container: HTMLElement
let clipboard: string[]
let toasts: ToastInput[]
let disconnectToasts: () => void

beforeEach(async () => {
  clipboard = []
  toasts = []
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const navigator = dom.window.navigator
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: async (text: string) => {
        clipboard.push(text)
      },
    },
  })
  const matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  })
  const globals = {
    window: Object.assign(dom.window, { matchMedia }),
    document: dom.document,
    navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    matchMedia,
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
  disconnectToasts = connectToastHost({
    show: (input) => {
      toasts.push(input)
      return String(toasts.length)
    },
    update: () => {},
  })
  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  disconnectToasts()
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

function button(name: string): Element {
  const match = [...container.querySelectorAll('button')].find(
    (el) => el.getAttribute('aria-label') === name,
  )
  if (!match) throw new Error(`missing button: ${name}`)
  return match
}

async function click(el: Element) {
  await act(async () => {
    el.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

const connected: ConnectionState = {
  harness: 'Codex',
  installed: true,
  configPath: '~/.codex/config.toml',
} as ConnectionState

describe('Connected agents copy actions', () => {
  it('copies an agent’s settings file path and says so', async () => {
    await act(async () =>
      root.render(
        <ul>
          <ConnectionRow
            state={connected}
            isPending={false}
            errorMessage={null}
            onConnect={() => {}}
            onDisconnect={() => {}}
          />
        </ul>,
      ),
    )

    await click(button('More for Codex'))
    await click(button('Copy where Codex’s settings file is'))

    expect(clipboard).toEqual(['~/.codex/config.toml'])
    expect(toasts).toEqual([
      {
        title: 'Copied to clipboard',
        description: '~/.codex/config.toml',
        status: 'success',
      },
    ])
  })

  it('shows neo’s MCP address and copies it', async () => {
    await act(async () =>
      root.render(<McpAddressCard endpointUrl="http://127.0.0.1:9200/mcp" />),
    )

    expect(container.querySelector('code')?.textContent).toBe(
      'http://127.0.0.1:9200/mcp',
    )
    await click(button('Copy neo’s MCP address'))

    expect(clipboard).toEqual(['http://127.0.0.1:9200/mcp'])
  })

  it('holds the address’s place while it resolves', async () => {
    await act(async () => root.render(<McpAddressCard endpointUrl={null} />))

    expect(
      container.querySelector('[role="status"]')?.getAttribute('aria-label'),
    ).toBe('Finding the address')
  })
})
