import { storage } from '@wxt-dev/storage'

export const declinedAppsStorage = storage.defineItem<string[]>(
  'local:declinedApps',
  { fallback: [] },
)
