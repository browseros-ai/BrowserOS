/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Utility for copying the conversation session ID to clipboard
 * and building a short display label for the UI.
 */

export const SESSION_ID_COPIED_EVENT = 'sidepanel.session_id.copied'

/**
 * Copies the conversation ID to the system clipboard.
 * Returns false for empty/null/undefined input without copying.
 */
export async function copySessionIdToClipboard(
  conversationId: string | null | undefined,
): Promise<undefined | false> {
  if (!conversationId) {
    return false
  }

  await navigator.clipboard.writeText(conversationId)
}

/**
 * Builds a short label for display.
 * Format: first N chars + "..." + last N chars (default N=4).
 * Returns "—" for null/undefined, "" for empty string.
 */
export function buildSessionIdLabel(
  conversationId: string | null | undefined,
  maxLength = 8,
): string {
  if (conversationId == null) return '—'
  if (conversationId === '') return ''

  if (conversationId.length <= maxLength) return conversationId

  // For default maxLength=8: head=4, tail=4 → "550e...0000" (length 11)
  // The label is a display label, not bounded by maxLength exactly;
  // maxLength controls the character count from the ID.
  const headLen = Math.max(Math.floor(maxLength / 2), 2)
  const tailLen = Math.max(Math.ceil(maxLength / 2), 2)
  const head = conversationId.slice(0, headLen)
  const tail = conversationId.slice(-tailLen)
  return `${head}...${tail}`
}
