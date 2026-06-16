/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Slug helpers, mirrored from the UI's wizard so the values the user
 * sees in the wizard preview match the slugs the server actually
 * persists. Keeping the logic identical (lowercase, non-alphanum
 * runs collapse to `-`, trim leading/trailing `-`, fall back to
 * `agent`) means we can rename the wizard preview into a stale-state
 * indicator instead of a "what the server will pick" guess.
 */

const MAX_COLLISION_SUFFIX = 99

export function toSlug(input: string): string {
  const cleaned = input
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
  return cleaned || 'agent'
}

/**
 * Returns a slug not present in `existing`. Appends `-2`, `-3`, ...,
 * up to MAX_COLLISION_SUFFIX. Throws if even the highest suffix is
 * taken (vanishingly unlikely; we surface 409 in the route layer if
 * it ever fires).
 */
export function uniqueSlug(
  base: string,
  existing: ReadonlySet<string>,
): string {
  if (!existing.has(base)) return base
  for (let i = 2; i <= MAX_COLLISION_SUFFIX; i++) {
    const candidate = `${base}-${i}`
    if (!existing.has(candidate)) return candidate
  }
  throw new Error(`slug-collision-exhausted: ${base}`)
}
