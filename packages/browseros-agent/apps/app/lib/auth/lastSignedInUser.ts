import { storage } from '@wxt-dev/storage'

/**
 * The most recent signed-in user id, persisted locally so their chat history
 * stays scoped to them after sign-out (issue #559) without leaking to another
 * account that signs into the same browser profile.
 */
export const lastSignedInUserStorage = storage.defineItem<string | null>(
  'local:lastSignedInUserId',
  { fallback: null },
)
