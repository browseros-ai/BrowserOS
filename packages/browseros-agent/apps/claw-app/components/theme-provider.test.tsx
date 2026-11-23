import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import {
  BROWSER_SCHEME_STORAGE_KEY,
  getCachedBrowserScheme,
  THEME_STORAGE_KEY,
} from '@/lib/theme/theme-storage'
import { ThemeProvider, useTheme } from './theme-provider'

class MemoryStorage {
  private store = new Map<string, string>()

  getItem(key: string): string | null {
    return this.store.get(key) ?? null
  }

  setItem(key: string, value: string): void {
    this.store.set(key, value)
  }

  removeItem(key: string): void {
    this.store.delete(key)
  }
}

const names = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'localStorage',
  'chrome',
  'IS_REACT_ACT_ENVIRONMENT',
]
const descriptors = new Map(
  names.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

const globals = globalThis as { localStorage?: MemoryStorage; chrome?: unknown }

/** kLight is 1, kDark is 2. Tests flip this after the first read. */
let prefValue = 2

let root: Root
let container: HTMLElement

function Probe() {
  const { theme, resolvedTheme, setTheme } = useTheme()
  return (
    <>
      <span data-theme={theme} />
      <span data-resolved={resolvedTheme} />
      <button
        type="button"
        data-set-theme="system"
        onClick={() => setTheme('system')}
      >
        Match browser
      </button>
    </>
  )
}

beforeEach(() => {
  prefValue = 2
  const storage = new MemoryStorage()
  storage.setItem(THEME_STORAGE_KEY, 'light')
  storage.setItem(BROWSER_SCHEME_STORAGE_KEY, 'dark')
  globals.localStorage = storage
  globals.chrome = {
    browserOS: {
      getPref: (name: string, callback: (pref: unknown) => void) => {
        callback({ key: name, type: 'number', value: prefValue })
      },
    },
    runtime: {},
  }

  const dom = parseHTML('<html><body><div id="root"></div></body></html>')
  const installed = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    localStorage: storage,
    chrome: globals.chrome,
    IS_REACT_ACT_ENVIRONMENT: true,
  }
  for (const [name, value] of Object.entries(installed)) {
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

async function flush() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

async function mount() {
  await act(async () => {
    root.render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    )
  })
  await flush()
}

function resolved(): string | null {
  return (
    container.querySelector('[data-resolved]')?.getAttribute('data-resolved') ??
    null
  )
}

describe('ThemeProvider', () => {
  it('applies the live browser scheme when Match browser is selected over a stale dark cache', async () => {
    // Match browser is off, and claw:browser-scheme is a leftover dark.
    // Appearance then flips to light with no visibility event. Selecting
    // Match browser must read that pref. Trusting the cache paints dark.
    await mount()

    expect(document.documentElement.classList.contains('light')).toBe(true)
    expect(getCachedBrowserScheme()).toBe('dark')

    prefValue = 1
    await act(async () => {
      container
        .querySelector('[data-set-theme="system"]')
        ?.dispatchEvent(new window.Event('click', { bubbles: true }))
      await new Promise((resolve) => setTimeout(resolve, 0))
    })

    expect(
      container.querySelector('[data-theme]')?.getAttribute('data-theme'),
    ).toBe('system')
    expect(resolved()).toBe('light')
    expect(document.documentElement.classList.contains('light')).toBe(true)
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(getCachedBrowserScheme()).toBe('light')
  })

  it('refreshes a stale scheme when the tab becomes visible in system mode', async () => {
    globals.localStorage?.setItem(THEME_STORAGE_KEY, 'system')
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => 'visible',
    })
    await mount()
    expect(resolved()).toBe('dark')
    expect(getCachedBrowserScheme()).toBe('dark')

    prefValue = 1
    await act(async () => {
      document.dispatchEvent(new window.Event('visibilitychange'))
      await new Promise((resolve) => setTimeout(resolve, 0))
    })

    expect(resolved()).toBe('light')
    expect(document.documentElement.classList.contains('light')).toBe(true)
    expect(getCachedBrowserScheme()).toBe('light')
  })

  it('follows the OS when Appearance Mode becomes Device over a stale dark cache', async () => {
    // Match browser + Mode Dark caches dark. Switching Mode to Device flips
    // prefers-color-scheme with the tab still visible — no visibility event.
    // Trusting the cache keeps dark and never attaches an OS listener.
    globals.localStorage?.setItem(THEME_STORAGE_KEY, 'system')

    let darkMatches = true
    const mediaListeners = new Set<(event: { matches: boolean }) => void>()
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      writable: true,
      value: (query: string) => ({
        get matches() {
          return query.includes('dark') ? darkMatches : !darkMatches
        },
        media: query,
        addEventListener: (
          _type: string,
          listener: (event: { matches: boolean }) => void,
        ) => {
          mediaListeners.add(listener)
        },
        removeEventListener: (
          _type: string,
          listener: (event: { matches: boolean }) => void,
        ) => {
          mediaListeners.delete(listener)
        },
        addListener: (listener: (event: { matches: boolean }) => void) => {
          mediaListeners.add(listener)
        },
        removeListener: (listener: (event: { matches: boolean }) => void) => {
          mediaListeners.delete(listener)
        },
        dispatchEvent: () => true,
        onchange: null,
      }),
    })

    await mount()
    expect(resolved()).toBe('dark')
    expect(getCachedBrowserScheme()).toBe('dark')

    // kSystem — Appearance Mode Device; OS is now light.
    prefValue = 0
    darkMatches = false
    await act(async () => {
      for (const listener of mediaListeners) {
        listener({ matches: false })
      }
      await new Promise((resolve) => setTimeout(resolve, 0))
    })

    expect(resolved()).toBe('light')
    expect(document.documentElement.classList.contains('light')).toBe(true)
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(getCachedBrowserScheme()).toBeNull()
  })
})
