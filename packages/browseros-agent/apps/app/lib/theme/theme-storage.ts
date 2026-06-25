import { storage } from '@wxt-dev/storage'

/**
 * @public
 */
export type Theme = 'light' | 'dark' | 'system'

/**
 * @public
 */
export const themeStorage = storage.defineItem<Theme>('local:theme', {
  fallback: 'system',
})
