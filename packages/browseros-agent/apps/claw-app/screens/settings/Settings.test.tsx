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
  SaveJevTokenRequest,
  UpdateAgentSettingsRequest,
} from '@browseros/claw-api'
import { JevTokenSource } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import * as posthog from '@/modules/analytics/posthog'
import * as api from '@/modules/api/client'

const settingsOf = (overrides: Partial<AgentSettings> = {}): AgentSettings => ({
  humanHelpEnabled: true,
  taskCleanupTipEnabled: false,
  jevTokenSource: JevTokenSource.None,
  ...overrides,
})

const server: {
  read: () => Promise<AgentSettings>
  save: (body: UpdateAgentSettingsRequest) => Promise<AgentSettings>
  saves: UpdateAgentSettingsRequest[]
  saveToken: (body: SaveJevTokenRequest) => Promise<AgentSettings>
  removeToken: () => Promise<AgentSettings>
  tokenSaves: SaveJevTokenRequest[]
  tokenRemovals: number
} = {
  read: async () => settingsOf(),
  save: async (body) => settingsOf(body),
  saves: [],
  saveToken: async () =>
    settingsOf({ jevTokenSource: JevTokenSource.Settings }),
  removeToken: async () => settingsOf(),
  tokenSaves: [],
  tokenRemovals: 0,
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
    saveJevToken: (body: SaveJevTokenRequest) => {
      server.tokenSaves.push(body)
      return server.saveToken(body)
    },
    removeJevToken: () => {
      server.tokenRemovals += 1
      return server.removeToken()
    },
  }),
}))

// Only the transport is replaced. The flag reader is the real one, driven
// through the same function PostHog's callback drives it with, because
// `mock.module` is process wide and a stubbed reader here would fight the
// module's own tests over which implementation is the real one.
mock.module('@/modules/analytics/posthog', () => ({
  ...posthog,
  capture: (event: string, properties?: Record<string, unknown>) => {
    captured.push({ event, properties })
  },
}))

/** What an `onFeatureFlags` arrival looks like, for one flag. */
function setFlag(key: string, enabled: boolean): void {
  posthog.applyFlags(enabled ? [key] : [])
}

const TOKEN_FIELD_FLAG = 'ultrafast-token-field'

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
    const toggle = switchIn(page, '#task-cleanup-tip-switch')
    expect(toggle.getAttribute('aria-checked')).toBe('false')
    expect(page.body.textContent).toContain(
      'When you finish this task, close your task’s tab group.',
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
    server.read = async () => settingsOf({ humanHelpEnabled: true })
    server.save = async (body) => settingsOf(body)
    server.saves = []
    server.saveToken = async () =>
      settingsOf({ jevTokenSource: JevTokenSource.Settings })
    server.removeToken = async () => settingsOf()
    server.tokenSaves = []
    server.tokenRemovals = 0
    captured.length = 0
    // The field is behind a flag; the tests below are about the field, and the
    // gate itself is tested separately.
    setFlag(TOKEN_FIELD_FLAG, true)

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
      await clickSwitch('#task-cleanup-tip-switch')

      expect(server.saves.at(-1)).toEqual({ taskCleanupTipEnabled })
      expect(saved).toMatchObject({ humanHelpEnabled, taskCleanupTipEnabled })
      expect(switchIn(container).getAttribute('aria-checked')).toBe(
        String(humanHelpEnabled),
      )
      expect(
        switchIn(container, '#task-cleanup-tip-switch').getAttribute(
          'aria-checked',
        ),
      ).toBe(String(taskCleanupTipEnabled))
    }
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

    server.save = async (body) => settingsOf(body)
    await clickSwitch()

    expect(saveError()).toBeUndefined()
    expect(switchIn(container).getAttribute('aria-checked')).toBe('false')
    expect(captured).toEqual([
      { event: 'human_help_toggled', properties: { enabled: false } },
    ])
  })
  // --- the decision model credential ---------------------------------------

  function tokenField(): HTMLInputElement | null {
    return container.querySelector(
      '#jev-token-input',
    ) as unknown as HTMLInputElement | null
  }

  function buttonNamed(label: string): HTMLElement | null {
    return (Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === label,
    ) ?? null) as unknown as HTMLElement | null
  }

  // The field is uncontrolled, so setting its value is all a user does.
  async function type(value: string) {
    const field = tokenField()
    if (!field) throw new Error('no token field')
    field.value = value
  }

  async function click(label: string) {
    const button = buttonNamed(label)
    if (!button) throw new Error(`no ${label} button`)
    await act(async () => {
      button.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    await settle()
  }

  async function submitToken() {
    const form = container.querySelector('form')
    if (!form) throw new Error('no token form')
    await act(async () => {
      form.dispatchEvent(new window.Event('submit', { bubbles: true }))
    })
    await settle()
  }

  it('tests that a server with no token offers a field to paste one into', async () => {
    await mount()

    expect(tokenField()).not.toBeNull()
    expect(tokenField()?.getAttribute('type')).toBe('password')
    expect(container.textContent).toContain(
      'Without one the tool is not offered',
    )
    expect(buttonNamed('Remove')).toBeNull()
  })

  it('tests that a blank field saves nothing rather than storing a blank', async () => {
    await mount()

    await submitToken()
    expect(server.tokenSaves).toEqual([])
    await type('   ')
    await submitToken()
    expect(server.tokenSaves).toEqual([])
  })

  it('tests that a pasted token is sent trimmed and the field is cleared', async () => {
    await mount()
    await type('  jev-a-token  ')
    await submitToken()

    expect(server.tokenSaves).toEqual([{ token: 'jev-a-token' }])
    // The reply said a token is saved, so the field gives way to it.
    expect(tokenField()).toBeNull()
    expect(container.textContent).toContain('A token is saved')
  })

  it('tests that a saved token can be removed', async () => {
    server.read = async () =>
      settingsOf({ jevTokenSource: JevTokenSource.Settings })
    await mount()

    expect(tokenField()).toBeNull()
    await click('Remove')

    expect(server.tokenRemovals).toBe(1)
    expect(tokenField()).not.toBeNull()
  })

  it('tests that replacing a saved token asks for the new one first', async () => {
    server.read = async () =>
      settingsOf({ jevTokenSource: JevTokenSource.Settings })
    await mount()
    await click('Replace')

    expect(tokenField()).not.toBeNull()
    expect(server.tokenRemovals).toBe(0)
    expect(server.tokenSaves).toEqual([])
  })

  /// The case a boolean could not have expressed, and the reason the reply
  /// carries a source instead.
  it('tests that a token in the environment is named rather than ignored', async () => {
    server.read = async () =>
      settingsOf({ jevTokenSource: JevTokenSource.Environment })
    await mount()

    expect(container.textContent).toContain('token in the server')
    expect(container.textContent).toContain('takes precedence')
    // And a token may still be pasted over it.
    expect(tokenField()).not.toBeNull()
  })

  it('tests that a failed token save says so and keeps what was typed', async () => {
    server.saveToken = async () => {
      throw new Error('nope')
    }
    await mount()
    await type('jev-a-token')
    await submitToken()

    expect(container.textContent).toContain('Could not save that')
    expect(tokenField()?.value).toBe('jev-a-token')
  })

  it('tests that a saved token is not rendered back into the page', async () => {
    const secret = 'not-a-real-token-0123456789'
    await mount()
    await type(secret)
    await submitToken()

    expect(server.tokenSaves).toEqual([{ token: secret }])
    // The field gave way to Replace and Remove, and nothing carries the value.
    expect(container.innerHTML).not.toContain(secret)
    expect(container.querySelector('[value]')).toBeNull()
  })
  // --- the flag that reveals the field -------------------------------------

  it('tests that the field is hidden when the flag is off', async () => {
    setFlag(TOKEN_FIELD_FLAG, false)
    await mount()

    expect(tokenField()).toBeNull()
    expect(buttonNamed('Save')).toBeNull()
    expect(container.textContent).not.toContain('Ultrafast decision loop')
    // The rest of the page is unaffected.
    expect(switchIn(container)).not.toBeNull()
  })

  /// The field must be absent on the first render rather than appear a beat
  /// later, which is the difference a user actually notices.
  it('tests that the field is hidden before any flag has been evaluated', async () => {
    setFlag(TOKEN_FIELD_FLAG, false)
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <Settings />
        </QueryClientProvider>,
      )
    })

    // Read before settling, so this is the first paint and not the steady state.
    expect(container.textContent).not.toContain('Ultrafast decision loop')
    await settle()
    expect(container.textContent).not.toContain('Ultrafast decision loop')
  })

  it('tests that the field appears when the flag turns on without a reload', async () => {
    setFlag(TOKEN_FIELD_FLAG, false)
    await mount()
    expect(tokenField()).toBeNull()

    // What onFeatureFlags arriving after the first render looks like.
    await act(async () => {
      setFlag(TOKEN_FIELD_FLAG, true)
    })
    await settle()

    expect(tokenField()).not.toBeNull()
  })

  /// A flag that could strand a saved credential would be worse than the
  /// thing it was hiding, so what the flag hides is only the field.
  it('tests that the flag does not touch anything but the field', async () => {
    server.read = async () =>
      settingsOf({ jevTokenSource: JevTokenSource.Settings })
    setFlag(TOKEN_FIELD_FLAG, false)
    await mount()

    // No way to remove it from here, which is the accepted cost, but the
    // switch still works and the server was never asked to change.
    expect(buttonNamed('Remove')).toBeNull()
    expect(server.tokenRemovals).toBe(0)
    await clickSwitch()
    expect(server.saves).toEqual([{ humanHelpEnabled: false }])
  })
})
