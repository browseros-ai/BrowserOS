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
  AuditCleanupResult,
  AuditStorageState,
  UpdateAgentSettingsRequest,
} from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import * as posthog from '@/modules/analytics/posthog'
import * as api from '@/modules/api/client'

const settingsOf = (overrides: Partial<AgentSettings> = {}): AgentSettings => ({
  humanHelpEnabled: true,
  taskCleanupTipEnabled: false,
  ...overrides,
})

const server: {
  read: () => Promise<AgentSettings>
  save: (body: UpdateAgentSettingsRequest) => Promise<AgentSettings>
  saves: UpdateAgentSettingsRequest[]
  storage: () => Promise<AuditStorageState>
  cleanup: () => Promise<AuditCleanupResult>
} = {
  read: async () => settingsOf(),
  save: async (body) => settingsOf(body),
  saves: [],
  storage: () => new Promise(() => {}),
  cleanup: async () => ({
    sessionsDeleted: 2,
    screenshotsDeleted: 0,
    recordingsDeleted: 0,
    bytesReclaimed: 0,
    usage: { recordingBytes: 0, screenshotBytes: 0, totalBytes: 0 },
  }),
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
    getAuditStorage: () => server.storage(),
    runAuditCleanup: () => server.cleanup(),
  }),
}))

mock.module('@/modules/analytics/posthog', () => ({
  ...posthog,
  capture: (event: string, properties?: Record<string, unknown>) => {
    captured.push({ event, properties })
  },
}))

// The beUI switch animates through motion, which linkedom cannot drive. A
// plain button keeps the props the screen relies on.
mock.module('@/components/motion/switch', () => ({
  Switch: ({
    checked,
    onCheckedChange,
    disabled,
    ariaLabel,
    ariaDescribedBy,
  }: {
    checked: boolean
    onCheckedChange: (checked: boolean) => void
    disabled?: boolean
    ariaLabel?: string
    ariaDescribedBy?: string
  }) => (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      aria-describedby={ariaDescribedBy}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
    />
  ),
}))

// The theme lives in a provider that reads browser storage; the screen only
// needs its value and setter.
mock.module('@/components/theme-provider', () => ({
  useTheme: () => ({
    theme: 'system',
    resolvedTheme: 'light',
    setTheme: () => undefined,
  }),
}))

const CLEANUP_SWITCH = '[aria-label="Tidy up when a task finishes"]'

const { useAgentSettings } = await import('@/modules/api/agent-settings.hooks')
const { useAuditAgents } = await import('@/modules/api/audit.hooks')
const { Settings } = await import('./Settings')

afterAll(() => mock.restore())

function renderPage(settings?: AgentSettings): Document {
  const client = new QueryClient()
  if (settings) client.setQueryData(useAgentSettings.getKey(), settings)
  const html = renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Settings />
      </MemoryRouter>
    </QueryClientProvider>,
  )
  const page = parseHTML(`<!doctype html><html><body>${html}</body></html>`)
  return page.document
}

function switchIn(
  page: Document | Element,
  selector = '[role="switch"]',
): Element {
  const element = page.querySelector(selector)
  if (!element) throw new Error('no switch')
  return element
}

describe('Settings screen', () => {
  it('tests that the cleanup tip switch is off by default and explains the tip', () => {
    const page = renderPage(settingsOf())
    const toggle = switchIn(page, CLEANUP_SWITCH)
    expect(toggle.getAttribute('aria-checked')).toBe('false')
    expect(page.body.textContent).toContain(
      'Your AI agents close the pages they opened once they’re done.',
    )
  })

  it.each([true, false])(
    'tests that human help reflects the saved value %p',
    (humanHelpEnabled) => {
      const toggle = switchIn(renderPage(settingsOf({ humanHelpEnabled })))
      expect(toggle.getAttribute('aria-checked')).toBe(String(humanHelpEnabled))
      expect(toggle.hasAttribute('disabled')).toBe(false)
    },
  )

  it('tests that the switch is named by its title and described by its explanation', () => {
    const page = renderPage(settingsOf({ humanHelpEnabled: true }))
    const toggle = switchIn(page)
    const textOf = (id: string | null) =>
      id ? page.getElementById(id)?.textContent : undefined

    expect(toggle.getAttribute('aria-label')).toBe('Ask me when it gets stuck')
    expect(textOf(toggle.getAttribute('aria-describedby'))).toBe(
      'If an agent needs you to sign in, type a code or solve a captcha, it waits and you get a notification. When this is off, agents ask you in their own app instead.',
    )
  })
})

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'Node',
    'Event',
    'ResizeObserver',
    'getComputedStyle',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

describe('Settings screen interactions', () => {
  let root: Root
  let container: HTMLElement
  let client: QueryClient

  beforeEach(async () => {
    server.read = async () => settingsOf({ humanHelpEnabled: true })
    server.save = async (body) => settingsOf(body)
    server.saves = []
    server.storage = () => new Promise(() => {})
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
      // linkedom has no layout; the theme tabs only measure with this.
      ResizeObserver: class {
        observe() {}
        disconnect() {}
      },
      getComputedStyle: () => ({ getPropertyValue: () => '' }),
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
          <MemoryRouter>
            <Settings />
          </MemoryRouter>
        </QueryClientProvider>,
      )
    })
    await settle()
  }

  async function clickSwitch(selector?: string) {
    await act(async () => {
      switchIn(container, selector).dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    await settle()
  }

  function saveError(): string | null | undefined {
    return container.querySelector('[role="alert"]')?.textContent
  }

  it('tests that cleaning up history also refreshes History’s agent list', async () => {
    // The confirm dialog animates; linkedom has no animation frames.
    Object.assign(globalThis, {
      requestAnimationFrame: (run: FrameRequestCallback) =>
        setTimeout(() => run(0), 0),
      cancelAnimationFrame: (id: number) => clearTimeout(id),
    })
    client.setQueryData(useAuditAgents.getKey(), {
      items: [
        { slug: 'codex', label: 'Codex', taskCount: 2, lastStartedAt: 1 },
      ],
    })
    await mount()

    // StatefulButton repeats its label in hidden sizing spans.
    const buttonNamed = (text: string) => {
      const match = [...document.querySelectorAll('button')].find((el) => {
        const label = el.textContent ?? ''
        return label !== '' && label.split(text).join('') === ''
      })
      if (!match) throw new Error(`missing button: ${text}`)
      return match
    }
    await act(async () => {
      buttonNamed('Clean up now').dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    await act(async () => {
      buttonNamed('Clean up').dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    await settle()

    expect(client.getQueryState(useAuditAgents.getKey())?.isInvalidated).toBe(
      true,
    )
    Reflect.deleteProperty(globalThis, 'requestAnimationFrame')
    Reflect.deleteProperty(globalThis, 'cancelAnimationFrame')
  })

  it('tests that both cleanup tip states preserve human-help changes made in another settings page', async () => {
    let saved = settingsOf()
    server.read = async () => ({ ...saved })
    server.save = async (body) => {
      saved = { ...saved, ...body }
      return saved
    }
    await mount()

    for (const [taskCleanupTipEnabled, humanHelpEnabled] of [
      [true, false],
      [false, true],
    ] as const) {
      // The other page changes human help after this page loaded its value.
      saved = { ...saved, humanHelpEnabled }
      await clickSwitch(CLEANUP_SWITCH)

      expect(server.saves.at(-1)).toEqual({ taskCleanupTipEnabled })
      expect(saved).toMatchObject({ humanHelpEnabled, taskCleanupTipEnabled })
      expect(switchIn(container).getAttribute('aria-checked')).toBe(
        String(humanHelpEnabled),
      )
      expect(
        switchIn(container, CLEANUP_SWITCH).getAttribute('aria-checked'),
      ).toBe(String(taskCleanupTipEnabled))
    }
  })

  it('tests that a saved custom history period shows its own days once it loads', async () => {
    let resolveStorage: (state: AuditStorageState) => void = () => {}
    server.storage = () =>
      new Promise((resolve) => {
        resolveStorage = resolve
      })
    await mount()

    await act(async () =>
      resolveStorage({
        retention: { mode: 'deleteAfterDays', days: 90 },
        usage: { recordingBytes: 0, screenshotBytes: 0, totalBytes: 0 },
      }),
    )
    await settle()

    const days = container.querySelector(
      'input[aria-label="Days to keep task history"]',
    ) as HTMLInputElement | null
    expect(days?.value).toBe('90')
  })

  it('tests that the switch is disabled until settings load', async () => {
    const read = Promise.withResolvers<AgentSettings>()
    server.read = () => read.promise
    await mount()

    expect(switchIn(container).hasAttribute('disabled')).toBe(true)
    // Until the server answers, the switch shows its default: on.
    expect(switchIn(container).getAttribute('aria-checked')).toBe('true')

    read.resolve(settingsOf({ humanHelpEnabled: false }))
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
      'Couldn’t load these settings. Check that BrowserOS is running, then open this page again.',
    )
    expect(
      container.querySelector('[aria-label="Ask me when it gets stuck"]'),
    ).toBeNull()
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

    save.resolve(settingsOf({ humanHelpEnabled: false }))
    await settle()

    expect(switchIn(container).hasAttribute('disabled')).toBe(false)
    expect(captured).toHaveLength(1)
  })

  it('tests that the switch and analytics follow the value the server saved', async () => {
    server.save = async () => settingsOf({ humanHelpEnabled: true })
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

    expect(saveError()).toBe('Couldn’t save that setting. Try again.')
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
    expect(saveError()).toBe('Couldn’t save that setting. Try again.')

    server.save = async (body) => settingsOf(body)
    await clickSwitch()

    expect(saveError()).toBeUndefined()
    expect(switchIn(container).getAttribute('aria-checked')).toBe('false')
    expect(captured).toEqual([
      { event: 'human_help_toggled', properties: { enabled: false } },
    ])
  })
})
