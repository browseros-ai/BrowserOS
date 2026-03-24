import { storage } from '@wxt-dev/storage'

export const openBrowserOSHomeOnStartupStorage = storage.defineItem<boolean>(
  'local:open-browseros-home-on-startup',
  {
    fallback: true,
  },
)
