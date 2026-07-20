import { storage } from '@wxt-dev/storage'

export const browserClawPromoDismissedStorage = storage.defineItem<boolean>(
  'local:browserClawPromoDismissed',
  { fallback: false },
)
