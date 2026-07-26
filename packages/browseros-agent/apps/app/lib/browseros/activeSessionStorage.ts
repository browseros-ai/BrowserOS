import type { UIMessage } from 'ai'

/**
 * Session storage key for the live conversation snapshot.
 * Written on sendMessage (synchronous path) and on every message change.
 * Cleared only when the user explicitly resets the conversation.
 */
export const SHIMMY_ACTIVE_SESSION_KEY = 'shimmyActiveSession' as const

/**
 * Flag set by the background script right before it migrates the side panel
 * to an agent-opened tab. The new panel reads this on mount to know it should
 * auto-restore. The flag is self-clearing after a short TTL.
 */
export const SHIMMY_RESTORE_PENDING_KEY = 'shimmyRestorePending' as const

export interface ActiveSession {
  conversationId: string
  messages: UIMessage[]
  /** Unix timestamp (ms) of the last write — used to detect stale sessions */
  updatedAt: number
}

/** Maximum age (ms) before a stored session is considered stale and ignored */
const MAX_SESSION_AGE_MS = 30 * 60 * 1000 // 30 minutes

/** How long the RESTORE_PENDING flag stays alive (ms) */
const RESTORE_PENDING_TTL_MS = 5000

/**
 * Persist the current live conversation to session storage.
 * Call this on every message change so other panels can restore.
 */
export function saveActiveSession(
  conversationId: string,
  messages: UIMessage[],
): void {
  if (messages.length === 0) return
  const session: ActiveSession = {
    conversationId,
    messages,
    updatedAt: Date.now(),
  }
  chrome.storage.session
    .set({ [SHIMMY_ACTIVE_SESSION_KEY]: session })
    .catch(() => null)
}

/**
 * Clear the stored live conversation (only called on explicit user reset).
 */
export function clearActiveSession(): void {
  chrome.storage.session
    .remove([SHIMMY_ACTIVE_SESSION_KEY, SHIMMY_RESTORE_PENDING_KEY])
    .catch(() => null)
}

/**
 * Read the stored live conversation, if any.
 * Returns `null` if none exists or if it is stale.
 */
export async function readActiveSession(): Promise<ActiveSession | null> {
  try {
    const data = await chrome.storage.session.get(SHIMMY_ACTIVE_SESSION_KEY)
    const session = data[SHIMMY_ACTIVE_SESSION_KEY] as ActiveSession | undefined
    if (!session) return null
    if (Date.now() - session.updatedAt > MAX_SESSION_AGE_MS) return null
    return session
  } catch {
    return null
  }
}

/**
 * Set the RESTORE_PENDING flag from the background script before opening
 * the side panel on an agent-navigated tab.
 * The flag auto-expires after RESTORE_PENDING_TTL_MS.
 */
export function markRestorePending(): void {
  const expiresAt = Date.now() + RESTORE_PENDING_TTL_MS
  chrome.storage.session
    .set({ [SHIMMY_RESTORE_PENDING_KEY]: expiresAt })
    .catch(() => null)
}

/**
 * Read and immediately clear the RESTORE_PENDING flag.
 * Returns true if a restore should happen (flag set and not expired).
 */
export async function consumeRestorePending(): Promise<boolean> {
  try {
    const data = await chrome.storage.session.get(SHIMMY_RESTORE_PENDING_KEY)
    const expiresAt = data[SHIMMY_RESTORE_PENDING_KEY] as number | undefined
    if (!expiresAt) return false
    await chrome.storage.session.remove(SHIMMY_RESTORE_PENDING_KEY)
    return Date.now() < expiresAt
  } catch {
    return false
  }
}
