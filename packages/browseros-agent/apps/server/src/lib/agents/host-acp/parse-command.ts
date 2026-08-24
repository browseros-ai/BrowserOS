/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * Split a custom agent's full command line into argv, honoring single and double
 * quotes (both literal). Backslashes are kept literal so Windows paths such as
 * `C:\Users\me\agent.exe` survive; use quotes for arguments containing spaces.
 * Throws on an unterminated quote so a malformed command is rejected up front
 * rather than spawned wrong.
 */
export function splitCommandLine(command: string): string[] {
  const argv: string[] = []
  let current = ''
  let quote: '"' | "'" | null = null
  let hasToken = false

  for (const ch of command) {
    if (quote) {
      if (ch === quote) quote = null
      else current += ch
      continue
    }

    if (ch === "'" || ch === '"') {
      quote = ch
      hasToken = true
      continue
    }

    if (ch === ' ' || ch === '\t' || ch === '\n' || ch === '\r') {
      if (hasToken) {
        argv.push(current)
        current = ''
        hasToken = false
      }
      continue
    }

    current += ch
    hasToken = true
  }

  if (quote) {
    throw new Error('Command has an unterminated quote')
  }
  if (hasToken) argv.push(current)
  return argv
}
