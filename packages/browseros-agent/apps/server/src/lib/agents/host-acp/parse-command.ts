/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * Split a custom agent's full command line into argv, honoring single quotes
 * (literal), double quotes (with backslash escapes), and backslash escapes
 * outside quotes. Throws on an unterminated quote so a malformed command is
 * rejected up front rather than spawned wrong.
 */
export function splitCommandLine(command: string): string[] {
  const argv: string[] = []
  let current = ''
  let quote: '"' | "'" | null = null
  let hasToken = false

  for (let i = 0; i < command.length; i += 1) {
    const ch = command[i]

    if (quote === "'") {
      if (ch === "'") quote = null
      else current += ch
      continue
    }

    if (quote === '"') {
      if (ch === '\\' && i + 1 < command.length) {
        i += 1
        current += command[i]
      } else if (ch === '"') {
        quote = null
      } else {
        current += ch
      }
      continue
    }

    if (ch === "'" || ch === '"') {
      quote = ch
      hasToken = true
      continue
    }

    if (ch === '\\' && i + 1 < command.length) {
      i += 1
      current += command[i]
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
