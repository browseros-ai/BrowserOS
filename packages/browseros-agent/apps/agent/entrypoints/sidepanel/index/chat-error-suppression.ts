/**
 * `useChat` from `@ai-sdk/react` keeps its `error` populated until a subsequent
 * request succeeds — there is no public clearError. When the user switches
 * provider or resets the conversation after a failure (e.g. credits exhausted,
 * invalid auth, malformed response) the error banner persists and gives the
 * impression the new provider also failed. Closes #862.
 *
 * The fix records the *reference* of the error at the moment the user takes a
 * recovery action; while `chatError` still points at that same reference we
 * treat it as stale and hide it from the UI. The next distinct error or a
 * cleared error invalidates the marker and the banner behaves normally again.
 */
export interface ChatErrorVisibilityInput {
  chatError: Error | undefined
  staleErrorMarker: Error | null
}

export function computeVisibleChatError(
  input: ChatErrorVisibilityInput,
): Error | undefined {
  if (!input.chatError) return undefined
  if (input.chatError === input.staleErrorMarker) return undefined
  return input.chatError
}

/**
 * Whether a stored marker is still relevant for the live error. Used by the
 * effect that drops the marker once `useChat` produces a different error
 * reference (new failure) or clears its error (successful request).
 */
export function isStaleErrorMarkerStillCurrent(
  chatError: Error | undefined,
  staleErrorMarker: Error | null,
): boolean {
  if (!staleErrorMarker) return false
  return chatError === staleErrorMarker
}
