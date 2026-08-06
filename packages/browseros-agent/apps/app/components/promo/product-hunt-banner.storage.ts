import { storage } from '@wxt-dev/storage'

export const productHuntBannerDismissedStorage = storage.defineItem<boolean>(
  'local:productHuntBannerDismissed',
  { fallback: false },
)
