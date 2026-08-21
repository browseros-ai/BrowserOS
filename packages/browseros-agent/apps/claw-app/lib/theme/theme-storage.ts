/// <reference path="../../modules/api/chrome-browser-os.d.ts" />

export type Theme = 'light' | 'dark' | 'system'

/** The two concrete schemes a preference can resolve to. */
export type ResolvedTheme = 'light' | 'dark'

export const THEME_STORAGE_KEY = 'claw:theme'

/**
 * Last known value of the browser's own colour-scheme pref. Cached
 * because reading the pref is async while the pre-render script in
 * index.html has to decide synchronously — see resolveTheme.
 */
export const BROWSER_SCHEME_STORAGE_KEY = 'claw:browser-scheme'

/**
 * chrome://settings Appearance -> Mode, i.e. ThemeService::BrowserColorScheme
 * (kSystem 0 / kLight 1 / kDark 2). BrowserOS patches the registered default
 * from kSystem to kLight in
 * packages/browseros/chromium_patches/chrome/browser/themes/theme_service.cc,
 * so the browser's scheme and the OS scheme are two independent switches:
 * a user on a dark OS still gets light chrome until they change this.
 * "System" in the cockpit therefore means the browser, not the OS.
 */
const BROWSER_COLOR_SCHEME_PREF = 'browser.theme.color_scheme2'
const BROWSER_COLOR_SCHEME_LIGHT = 1
const BROWSER_COLOR_SCHEME_DARK = 2

const themes: readonly Theme[] = ['light', 'dark', 'system']

export function normalizeTheme(value: unknown): Theme {
  return themes.includes(value as Theme) ? (value as Theme) : 'system'
}

/** Narrows a stored/broadcast browser scheme; null means "follow the OS". */
export function normalizeBrowserScheme(value: unknown): ResolvedTheme | null {
  return value === 'light' || value === 'dark' ? value : null
}

/**
 * localStorage over chrome.storage so the theme also works when the
 * build is served as a plain web page (dev:web); absent in bun tests
 * and may throw in sandboxed frames, hence the guard.
 */
function safeStorage(): Pick<
  Storage,
  'getItem' | 'setItem' | 'removeItem'
> | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

export function getStoredTheme(): Theme {
  return normalizeTheme(safeStorage()?.getItem(THEME_STORAGE_KEY))
}

export function setStoredTheme(theme: Theme): void {
  safeStorage()?.setItem(THEME_STORAGE_KEY, theme)
}

export function getCachedBrowserScheme(): ResolvedTheme | null {
  return normalizeBrowserScheme(
    safeStorage()?.getItem(BROWSER_SCHEME_STORAGE_KEY),
  )
}

export function setCachedBrowserScheme(scheme: ResolvedTheme | null): void {
  const storage = safeStorage()
  if (!storage) return
  if (scheme) storage.setItem(BROWSER_SCHEME_STORAGE_KEY, scheme)
  else storage.removeItem(BROWSER_SCHEME_STORAGE_KEY)
}

/**
 * Asks the browser which colour scheme its own chrome is in. Returns
 * null when the answer is "follow the OS" (kSystem) and when the pref
 * is unreachable — outside BrowserOS, in dev:web, and in bun tests.
 * Mirrors readBrowserOSPort in modules/api/browseros-ports.ts.
 */
export async function readBrowserColorScheme(): Promise<ResolvedTheme | null> {
  if (
    typeof chrome === 'undefined' ||
    typeof chrome.browserOS?.getPref !== 'function'
  ) {
    return null
  }

  try {
    const pref = await new Promise<chrome.browserOS.PrefObject>(
      (resolve, reject) => {
        chrome.browserOS.getPref(BROWSER_COLOR_SCHEME_PREF, (value) => {
          const message = chrome.runtime?.lastError?.message
          if (message) {
            reject(new Error(message))
            return
          }
          resolve(value)
        })
      },
    )
    if (pref.value === BROWSER_COLOR_SCHEME_LIGHT) return 'light'
    if (pref.value === BROWSER_COLOR_SCHEME_DARK) return 'dark'
    return null
  } catch {
    return null
  }
}

function prefersDarkScheme(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-color-scheme: dark)').matches
  )
}

/**
 * Resolves a preference to a concrete scheme. Synchronous, because the
 * pre-render script in index.html has to stamp the class before the
 * first style resolution.
 *
 * 'system' follows the browser's own scheme when we know it; the cached
 * value is the default so the boot script needs no arguments. Only when
 * the browser is itself on "System" (or is not BrowserOS) do we fall
 * through to prefers-color-scheme, which tracks the OS.
 */
export function resolveTheme(
  theme: Theme,
  browserScheme: ResolvedTheme | null = getCachedBrowserScheme(),
): ResolvedTheme {
  if (theme !== 'system') return theme
  if (browserScheme) return browserScheme
  return prefersDarkScheme() ? 'dark' : 'light'
}
