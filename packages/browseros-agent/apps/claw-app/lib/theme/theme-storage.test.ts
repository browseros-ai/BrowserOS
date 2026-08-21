import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  BROWSER_SCHEME_STORAGE_KEY,
  getCachedBrowserScheme,
  getStoredTheme,
  normalizeBrowserScheme,
  normalizeTheme,
  readBrowserColorScheme,
  resolveTheme,
  setCachedBrowserScheme,
  setStoredTheme,
  THEME_STORAGE_KEY,
} from './theme-storage'

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

const globals = globalThis as { localStorage?: unknown; chrome?: unknown }

/** Stubs the pref call the way BrowserOS answers it: kLight 1, kDark 2. */
function stubBrowserPref(value: unknown): void {
  globals.chrome = {
    browserOS: {
      getPref: (name: string, callback: (pref: unknown) => void) => {
        callback({ key: name, type: typeof value, value })
      },
    },
    runtime: {},
  }
}

describe('theme-storage', () => {
  beforeEach(() => {
    globals.localStorage = new MemoryStorage()
  })

  afterEach(() => {
    delete globals.localStorage
    delete globals.chrome
  })

  it('round-trips each theme through set/get', () => {
    for (const theme of ['light', 'dark', 'system'] as const) {
      setStoredTheme(theme)
      expect(getStoredTheme()).toBe(theme)
    }
  })

  it('persists under the claw:theme key', () => {
    setStoredTheme('dark')
    expect(
      (globals.localStorage as MemoryStorage).getItem(THEME_STORAGE_KEY),
    ).toBe('dark')
  })

  it('falls back to system when the key is unset', () => {
    expect(getStoredTheme()).toBe('system')
  })

  it('falls back to system for junk stored values', () => {
    for (const junk of ['blue', '', 'DARK']) {
      ;(globals.localStorage as MemoryStorage).setItem(THEME_STORAGE_KEY, junk)
      expect(getStoredTheme()).toBe('system')
    }
  })

  it('is safe without a localStorage global', () => {
    delete globals.localStorage
    expect(getStoredTheme()).toBe('system')
    expect(() => setStoredTheme('dark')).not.toThrow()
    expect(getCachedBrowserScheme()).toBeNull()
    expect(() => setCachedBrowserScheme('dark')).not.toThrow()
  })

  it('normalizes arbitrary values', () => {
    expect(normalizeTheme('dark')).toBe('dark')
    expect(normalizeTheme('light')).toBe('light')
    expect(normalizeTheme(null)).toBe('system')
    expect(normalizeTheme(42)).toBe('system')
  })

  it('normalizes browser schemes, treating anything else as unknown', () => {
    expect(normalizeBrowserScheme('dark')).toBe('dark')
    expect(normalizeBrowserScheme('light')).toBe('light')
    expect(normalizeBrowserScheme('system')).toBeNull()
    expect(normalizeBrowserScheme(null)).toBeNull()
  })

  it('resolves explicit themes as-is and system to light without a window', () => {
    expect(resolveTheme('dark')).toBe('dark')
    expect(resolveTheme('light')).toBe('light')
    expect(resolveTheme('system')).toBe('light')
  })

  it('resolves system against the browser scheme, not the OS', () => {
    expect(resolveTheme('system', 'dark')).toBe('dark')
    expect(resolveTheme('system', 'light')).toBe('light')
  })

  it('never lets the browser scheme override an explicit preference', () => {
    expect(resolveTheme('light', 'dark')).toBe('light')
    expect(resolveTheme('dark', 'light')).toBe('dark')
  })

  it('round-trips the cached browser scheme and clears it on null', () => {
    setCachedBrowserScheme('dark')
    expect(getCachedBrowserScheme()).toBe('dark')
    expect(resolveTheme('system')).toBe('dark')

    setCachedBrowserScheme(null)
    expect(getCachedBrowserScheme()).toBeNull()
    expect(
      (globals.localStorage as MemoryStorage).getItem(
        BROWSER_SCHEME_STORAGE_KEY,
      ),
    ).toBeNull()
  })

  it('ignores a junk cached browser scheme', () => {
    ;(globals.localStorage as MemoryStorage).setItem(
      BROWSER_SCHEME_STORAGE_KEY,
      'gray',
    )
    expect(getCachedBrowserScheme()).toBeNull()
  })

  it('reads kLight and kDark off the browser pref', async () => {
    stubBrowserPref(1)
    expect(await readBrowserColorScheme()).toBe('light')
    stubBrowserPref(2)
    expect(await readBrowserColorScheme()).toBe('dark')
  })

  it('returns null for kSystem so the OS decides', async () => {
    stubBrowserPref(0)
    expect(await readBrowserColorScheme()).toBeNull()
  })

  it('returns null when the browserOS pref API is absent', async () => {
    expect(await readBrowserColorScheme()).toBeNull()
    globals.chrome = {}
    expect(await readBrowserColorScheme()).toBeNull()
  })

  it('returns null when the pref call reports a runtime error', async () => {
    globals.chrome = {
      browserOS: {
        getPref: (_name: string, callback: (pref: unknown) => void) => {
          callback(undefined)
        },
      },
      runtime: { lastError: { message: 'Preference not found' } },
    }
    expect(await readBrowserColorScheme()).toBeNull()
  })
})
