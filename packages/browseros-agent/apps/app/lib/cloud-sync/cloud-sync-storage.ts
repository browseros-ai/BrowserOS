import { storage } from '#imports'

/**
 * One-time announcement, so dismissal has to outlive the session.
 *
 * Keyed by which announcement it was. The first notice said account chats
 * would stay visible in history, which stopped being true when sign-in was
 * removed; someone who dismissed that one would otherwise never be told the
 * part that actually affects them.
 */
export const cloudSyncNoticeDismissedStorage = storage.defineItem<boolean>(
  'local:accountRetiredNoticeDismissed',
  { fallback: false },
)
