import { storage } from '#imports'

/** One-time announcement, so dismissal has to outlive the session. */
export const cloudSyncNoticeDismissedStorage = storage.defineItem<boolean>(
  'local:cloudSyncNoticeDismissed',
  { fallback: false },
)
