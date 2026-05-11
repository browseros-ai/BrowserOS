import { storage } from '@wxt-dev/storage'

/**
 * Saved custom UI accent (hex `#rrggbb`). `null` = use built-in stylesheet theme.
 * @public
 */
export const accentThemeStorage = storage.defineItem<string | null>(
  'local:theme-accent-hex',
  {
    fallback: null,
  },
)
