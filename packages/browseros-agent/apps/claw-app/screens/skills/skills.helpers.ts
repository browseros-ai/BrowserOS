/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure formatters for the Tasks list and detail. No React, no side effects.
 */

/** The command a user pastes into a coding agent to run a skill. */
export function skillCommand(name: string): string {
  return `/${name}`
}

/** Compact token count, e.g. 14600 -> "14.6k". */
export function formatTokens(tokens: number): string {
  if (tokens < 1000) {
    return String(tokens)
  }
  return `${(tokens / 1000).toFixed(1)}k`
}

/**
 * Percent change from the first run's tokens to the latest run's, so a negative
 * value means the skill got cheaper. `null` when there is nothing to compare.
 */
export function tokenDeltaPercent(
  first: number | undefined,
  latest: number | undefined,
): number | null {
  if (first === undefined || latest === undefined || first === 0) {
    return null
  }
  return Math.round(((latest - first) / first) * 100)
}

/** A short "time ago" label from an epoch-ms timestamp. */
export function formatRelativeTime(
  epochMs: number,
  now: number = Date.now(),
): string {
  const minutes = Math.floor(Math.max(0, now - epochMs) / 60_000)
  if (minutes < 1) {
    return 'just now'
  }
  if (minutes < 60) {
    return `${minutes}m ago`
  }
  const hours = Math.floor(minutes / 60)
  if (hours < 24) {
    return `${hours}h ago`
  }
  return `${Math.floor(hours / 24)}d ago`
}
