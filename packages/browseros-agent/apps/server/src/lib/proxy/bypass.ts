/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * NO_PROXY / ProxyOverride bypass matching.
 *
 * One matcher serves env NO_PROXY entries, Windows ProxyOverride entries, and
 * macOS ExceptionsList entries. Windows uses `;` separators and a `<local>`
 * token; those are normalized by the platform readers before reaching here.
 */

/** Always bypassed: local LLM servers (Ollama/LMStudio) must stay direct. */
export const DEFAULT_BYPASS = ['localhost', '127.0.0.1', '::1']

function stripBrackets(host: string): string {
  return host.startsWith('[') && host.endsWith(']') ? host.slice(1, -1) : host
}

function patternMatchesHost(pattern: string, hostname: string): boolean {
  if (pattern === '*') return true
  // `<local>` (Windows): plain hostnames without a dot.
  if (pattern === '<local>') return !hostname.includes('.')
  let name = pattern.toLowerCase()
  if (name.startsWith('*.')) name = name.slice(2)
  else if (name.startsWith('.')) name = name.slice(1)
  if (!name) return false
  return hostname === name || hostname.endsWith(`.${name}`)
}

/**
 * Returns true when the target should bypass the proxy.
 * Patterns may be plain hosts, `.suffix`/`*.suffix` wildcards, `*`,
 * `<local>`, or `host:port` pairs (port must match when present).
 */
export function isBypassed(
  hostname: string,
  port: string,
  patterns: readonly string[],
): boolean {
  const host = stripBrackets(hostname.toLowerCase())
  for (const raw of patterns) {
    const pattern = raw.trim()
    if (!pattern) continue
    // Split host:port without breaking IPv6 literals.
    let name = pattern
    let patternPort: string | undefined
    const bracketed = /^\[.*\](?::(\d+))?$/.exec(pattern)
    const lastColon = pattern.lastIndexOf(':')
    if (bracketed) {
      name = pattern.slice(
        0,
        pattern.length - (bracketed[1] ? bracketed[1].length + 1 : 0),
      )
      patternPort = bracketed[1]
    } else if (lastColon > -1 && pattern.indexOf(':') === lastColon) {
      name = pattern.slice(0, lastColon)
      patternPort = pattern.slice(lastColon + 1)
    }
    if (patternPort && patternPort !== port) continue
    if (patternMatchesHost(stripBrackets(name.trim().toLowerCase()), host)) {
      return true
    }
  }
  return false
}
