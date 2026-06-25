/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Glob matcher for site-rule domain patterns. Patterns the user can
 * configure today:
 *
 *   stripe.com         exact match (case insensitive)
 *   *.example.com      any subdomain (must have at least one label
 *                      before .example.com; "example.com" does NOT
 *                      match)
 *   admin.*            any domain that starts with "admin."; "admin."
 *                      alone does not match (we require at least one
 *                      char after the dot)
 *   *                  matches every non-empty domain
 *
 * Same matcher is reused by Phase 3's executor pre-flight and Phase 6's
 * grants short-circuit, so the semantics live in one place and the
 * unit tests pin every shape.
 */

const REGEX_META = /[.+?^${}()|[\]\\]/g

export function matchDomain(pattern: string, domain: string): boolean {
  if (!pattern || !domain) return false
  const lowerPattern = pattern.toLowerCase()
  const lowerDomain = domain.toLowerCase()
  // Bare `*` is a hot path: matches any non-empty domain.
  if (lowerPattern === '*') return lowerDomain.length > 0
  // Translate the glob to a regex by escaping every metacharacter
  // EXCEPT `*`, then replacing each `*` with `.+`. `.+` (not `.*`) so
  // `*.foo.com` requires at least one label before `.foo.com`.
  const regexSource = lowerPattern
    .split('*')
    .map((part) => part.replace(REGEX_META, '\\$&'))
    .join('.+')
  return new RegExp(`^${regexSource}$`).test(lowerDomain)
}
