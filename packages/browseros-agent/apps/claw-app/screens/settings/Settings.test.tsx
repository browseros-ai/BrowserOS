/**
 * The Settings screen against a stubbed claw-server. The real react-query
 * hooks run; only the HTTP client, the analytics transport, and the switch
 * primitive are replaced.
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
  AgentSettings,
  UpdateAgentSettingsRequest,
} from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import * as api from '@/modules/api/client'

const server: {
  read: () => Promise<AgentSettings>
  save: (body: UpdateAgentSettingsRequest) => Promise<AgentSettings>
  saves: UpdateAgentSettingsRequest[]
} = {
  read: async () => ({ humanHelpEnabled: true }),
  save: async (body) => body,
  saves: [],
}
const captured: { event: string; properties?: Record<string, unknown> }[] = []

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getAgentSettings: () => server.read(),
    updateAgentSettings: (body: UpdateAgentSettingsRequest) => {
      server.saves.push(body)
      return server.save(body)
    },
  }),
}))

mock.module('@/modules/analytics/posthog', () => ({
  capture: (event: string, properties?: Record<string, unknown>) => {
    captured.push({ event, properties })
  },
}))

// base-ui's switch toggles a hidden checkbox with a PointerEvent, which
// linkedom lacks. A plain button keeps the props the screen relies on.
mock.module('@/components/ui/switch', () => ({
  Switch: ({
    checked,
    onCheckedChange,
    ...props
  }: ComponentProps<'button'> & {
    checked: boolean
    onCheckedChange: (checked: boolean) => void
  }) => (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onCheckedChange(!checked)}
      {...props}
    />
  ),
}))

const { useAgentSettings } = await import('@/modules/api/agent-settings.hooks')
const { Settings } = await import('./Settings')

afterAll(() => mock.restore())

function renderPage(settings?: AgentSettings): Document {
  const client = new QueryClient()
  if (settings) client.setQueryData(useAgentSettings.getKey(), settings)
  const html = renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <Settings />
    </QueryClientProvider>,
  )
  const page = parseHTML(`<!doctype html><html><body>${html}</body></html>`)
  return page.document
}

function switchIn(page: Document | Element): Element {
  const element = page.querySelector('[role="switch"]')
  if (!element) throw new Error('no switch')
  return element
}

describe('Settings screen', () => {
  it('tests that the switch is on when the server has human help on', () => {
    const toggle = switchIn(renderPage({ humanHelpEnabled: true }))

    expect(toggle.getAttribute('aria-checked')).toBe('true')
    expect(toggle.hasAttribute('disabled')).toBe(false)
  })

  it('tests that the switch is off when the server has human help off', () => {
    const toggle = switchIn(renderPage({ humanHelpEnabled: false }))

    expect(toggle.getAttribute('aria-checked')).toBe('false')
    expect(toggle.hasAttribute('disabled')).toBe(false)
  })

  it('tests that the switch is named by its title and described by its explanation', () => {
    const page = renderPage({ humanHelpEnabled: true })
    const toggle = switchIn(page)
    const textOf = (id: string | null) =>
      id ? page.getElementById(id)?.textContent : undefined

    expect(page.querySelector(`label[for="${toggle.id}"]`)).not.toBeNull()
    expect(textOf(toggle.getAttribute('aria-labelledby'))).toBe(
      'Ask for human help',
    )
    expect(textOf(toggle.getAttribute('aria-describedby'))).toBe(
      'Lets agents pause and ask you to take over the page for a sign-in, a code, or a captcha. When off, agents tell you what they need in their own chat instead.',
    )
  })
})

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

describe('Settings screen interactions', () => {
  let root: Root
  let container: HTMLElement
  let client: QueryClient

  beforeEach(async () => {
    server.read = async () => ({ humanHelpEnabled: true })
    server.save = async (body) => body
    server.saves = []
    captured.length = 0

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

  // react-query hands results to React on 0 ms timers; let them run in act.
  async function settle() {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }

  async function mount() {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <Settings />
        </QueryClientProvider>,
      )
    })
    await settle()
  }

  async function clickSwitch() {
    await act(async () => {
      switchIn(container).dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    await settle()
  }

  function saveError(): string | null | undefined {
    return container.querySelector('[role="alert"]')?.textContent
  }

  it('tests that the switch is disabled until settings load', async () => {
    const read = Promise.withResolvers<AgentSettings>()
    server.read = () => read.promise
    await mount()

    expect(switchIn(container).hasAttribute('disabled')).toBe(true)
    // Until the server answers, the switch shows its default: on.
    expect(switchIn(container).getAttribute('aria-checked')).toBe('true')

    read.resolve({ humanHelpEnabled: false })
    await settle()

    expect(switchIn(container).hasAttribute('disabled')).toBe(false)
    expect(switchIn(container).getAttribute('aria-checked')).toBe('false')
  })

  it('tests that a load error shows a notice instead of the switch', async () => {
    server.read = async () => {
      throw new TypeError('Failed to fetch')
    }
    await mount()

    expect(container.textContent).toContain(
      'Could not load settings. Check that BrowserOS neo is running and try again.',
    )
    expect(container.querySelector('[role="switch"]')).toBeNull()
  })

  it('tests that turning the switch off saves it and tracks the saved value', async () => {
    await mount()
    await clickSwitch()

    expect(server.saves).toEqual([{ humanHelpEnabled: false }])
    expect(switchIn(container).getAttribute('aria-checked')).toBe('false')
    expect(captured).toEqual([
      { event: 'human_help_toggled', properties: { enabled: false } },
    ])
  })

  it('tests that the switch is disabled and nothing is tracked while a save is pending', async () => {
    const save = Promise.withResolvers<AgentSettings>()
    server.save = () => save.promise
    await mount()
    await clickSwitch()

    expect(switchIn(container).hasAttribute('disabled')).toBe(true)
    expect(captured).toEqual([])

    save.resolve({ humanHelpEnabled: false })
    await settle()

    expect(switchIn(container).hasAttribute('disabled')).toBe(false)
    expect(captured).toHaveLength(1)
  })

  it('tests that the switch and analytics follow the value the server saved', async () => {
    server.save = async () => ({ humanHelpEnabled: true })
    await mount()
    await clickSwitch()

    expect(server.saves).toEqual([{ humanHelpEnabled: false }])
    expect(switchIn(container).getAttribute('aria-checked')).toBe('true')
    expect(captured).toEqual([
      { event: 'human_help_toggled', properties: { enabled: true } },
    ])
  })

  it('tests that a failed save shows an error, keeps the server value, and tracks nothing', async () => {
    server.save = async () => {
      throw new Error('internal_error')
    }
    await mount()
    await clickSwitch()

    expect(saveError()).toBe('Could not save this setting. Try again.')
    expect(switchIn(container).getAttribute('aria-checked')).toBe('true')
    expect(switchIn(container).hasAttribute('disabled')).toBe(false)
    expect(captured).toEqual([])
  })

  it('tests that retrying after a failed save clears the error', async () => {
    server.save = async () => {
      throw new Error('internal_error')
    }
    await mount()
    await clickSwitch()
    expect(saveError()).toBe('Could not save this setting. Try again.')

    server.save = async (body) => body
    await clickSwitch()

    expect(saveError()).toBeUndefined()
    expect(switchIn(container).getAttribute('aria-checked')).toBe('false')
    expect(captured).toEqual([
      { event: 'human_help_toggled', properties: { enabled: false } },
    ])
  })
})
