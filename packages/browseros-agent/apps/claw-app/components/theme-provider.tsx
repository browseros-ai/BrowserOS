import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react'
import {
  BROWSER_SCHEME_STORAGE_KEY,
  getCachedBrowserScheme,
  getStoredTheme,
  normalizeBrowserScheme,
  normalizeTheme,
  type ResolvedTheme,
  readBrowserColorScheme,
  resolveTheme,
  setCachedBrowserScheme,
  setStoredTheme,
  THEME_STORAGE_KEY,
  type Theme,
} from '@/lib/theme/theme-storage'

interface ThemeProviderState {
  /** The stored preference: what the toggle shows. */
  theme: Theme
  /** What that preference currently resolves to: what the page renders. */
  resolvedTheme: ResolvedTheme
  setTheme: (theme: Theme) => void
}

const ThemeProviderContext = createContext<ThemeProviderState | null>(null)

/**
 * Applies the resolved dark/light class to documentElement and keeps
 * it in sync with the browser's own colour scheme (in system mode) and
 * with other open tabs via storage events. Initial state is read
 * synchronously so the first render already matches the class the
 * index.html script set.
 */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(getStoredTheme)
  const [browserScheme, setBrowserScheme] = useState<ResolvedTheme | null>(
    getCachedBrowserScheme,
  )

  // Ask the browser what its own chrome is set to. getPref is callback-
  // based, so the answer lands after first paint; caching it means the
  // next paint starts from the right class instead of guessing off the
  // OS. There is no pref-change event, so re-read whenever the tab comes
  // back into view — flipping Appearance in chrome://settings and
  // returning to an open cockpit is exactly how someone tries gray mode.
  useEffect(() => {
    let cancelled = false

    const sync = () => {
      void readBrowserColorScheme().then((scheme) => {
        if (cancelled) return
        setCachedBrowserScheme(scheme)
        setBrowserScheme(scheme)
      })
    }
    const onVisibility = () => {
      if (document.visibilityState === 'visible') sync()
    }

    sync()
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      cancelled = true
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [])

  const resolved = resolveTheme(theme, browserScheme)

  useEffect(() => {
    const root = document.documentElement

    const apply = () => {
      root.classList.remove('light', 'dark')
      root.classList.add(resolveTheme(theme, browserScheme))
    }
    apply()

    // Only follow the OS while nothing more specific is speaking: an
    // explicit preference wins, and so does the browser's own scheme.
    if (theme !== 'system' || browserScheme) return
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
    mediaQuery.addEventListener('change', apply)
    return () => mediaQuery.removeEventListener('change', apply)
  }, [theme, browserScheme])

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === THEME_STORAGE_KEY) {
        setThemeState(normalizeTheme(event.newValue))
      } else if (event.key === BROWSER_SCHEME_STORAGE_KEY) {
        setBrowserScheme(normalizeBrowserScheme(event.newValue))
      }
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [])

  const setTheme = useCallback((next: Theme) => {
    setStoredTheme(next)
    setThemeState(next)
  }, [])

  const value = useMemo(
    () => ({ theme, resolvedTheme: resolved, setTheme }),
    [theme, resolved, setTheme],
  )

  return (
    <ThemeProviderContext.Provider value={value}>
      {children}
    </ThemeProviderContext.Provider>
  )
}

export function useTheme(): ThemeProviderState {
  const context = useContext(ThemeProviderContext)
  if (!context) throw new Error('useTheme must be used within a ThemeProvider')
  return context
}
