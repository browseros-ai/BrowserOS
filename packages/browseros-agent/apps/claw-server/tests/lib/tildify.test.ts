/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { homedir } from 'node:os'
import { tildifyHomePath } from '../../src/lib/tildify'

const HOME = homedir()

describe('tildifyHomePath', () => {
  test('replaces the home prefix with ~ for paths under home', () => {
    expect(tildifyHomePath(`${HOME}/.config/zed/settings.json`)).toBe(
      '~/.config/zed/settings.json',
    )
    expect(tildifyHomePath(`${HOME}/.claude.json`)).toBe('~/.claude.json')
  })

  test('returns "~" when the input is exactly the home dir', () => {
    expect(tildifyHomePath(HOME)).toBe('~')
  })

  test('passes non-home paths through unchanged', () => {
    expect(tildifyHomePath('/tmp/stub-cursor.json')).toBe(
      '/tmp/stub-cursor.json',
    )
    expect(tildifyHomePath('/etc/mcp.json')).toBe('/etc/mcp.json')
  })

  test('returns undefined for undefined input', () => {
    expect(tildifyHomePath(undefined)).toBeUndefined()
  })

  test('does not tildify a path that starts with the home dir but is a longer sibling', () => {
    // Guard against '/Users/dani' matching '/Users/danielle/...' style
    // false positives. The helper only swaps on exact match or when
    // followed by a slash.
    const sibling = `${HOME}extra/oops.json`
    expect(tildifyHomePath(sibling)).toBe(sibling)
  })
})
