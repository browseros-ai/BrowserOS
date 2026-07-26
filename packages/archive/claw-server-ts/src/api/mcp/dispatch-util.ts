/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/** Combines defined cancellation signals without wrapping the common single-signal case. */
export function composeAbortSignals(
  signals: ReadonlyArray<AbortSignal | undefined>,
): AbortSignal | undefined {
  const defined = signals.filter((signal): signal is AbortSignal =>
    Boolean(signal),
  )
  if (defined.length === 0) return undefined
  if (defined.length === 1) return defined[0]
  return AbortSignal.any(defined)
}

const DISPATCH_ERROR_TEXT_MAX = 200

/** Returns the bounded first text block used in failed-dispatch logs. */
export function dispatchErrorText(content: unknown): string | null {
  if (!Array.isArray(content)) return null
  for (const block of content) {
    if (
      block !== null &&
      typeof block === 'object' &&
      (block as { type?: unknown }).type === 'text' &&
      typeof (block as { text?: unknown }).text === 'string'
    ) {
      return (block as { text: string }).text.slice(0, DISPATCH_ERROR_TEXT_MAX)
    }
  }
  return null
}
