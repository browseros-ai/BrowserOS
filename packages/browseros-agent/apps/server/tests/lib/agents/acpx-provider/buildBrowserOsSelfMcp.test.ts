/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { buildBrowserOsSelfMcpEntry } from '../../../../src/lib/agents/acpx-provider/buildBrowserOsSelfMcp'

describe('buildBrowserOsSelfMcpEntry', () => {
  it('points the spawned agent at the local /mcp route on the given port', () => {
    const entry = buildBrowserOsSelfMcpEntry({
      serverPort: 9100,
      conversationId: 'conv-1',
      providerId: 'claude-code',
    })
    expect(entry.type).toBe('http')
    expect(entry.name).toBe('browseros')
    if (entry.type !== 'http') throw new Error('expected http entry')
    expect(entry.url).toBe('http://127.0.0.1:9100/mcp')
  })

  it('always sets the scope and agent headers', () => {
    const entry = buildBrowserOsSelfMcpEntry({
      serverPort: 9100,
      conversationId: 'conv-1',
      providerId: 'codex',
    })
    if (entry.type !== 'http') throw new Error('expected http entry')
    const header = (name: string) =>
      entry.headers.find((h) => h.name === name)?.value
    expect(header('X-BrowserOS-Scope-Id')).toBe('conv-1')
    expect(header('X-BrowserOS-Agent-Id')).toBe('codex')
  })

  it('omits the window-id header when no defaultWindowId is given', () => {
    const entry = buildBrowserOsSelfMcpEntry({
      serverPort: 9100,
      conversationId: 'conv-1',
      providerId: 'claude-code',
    })
    if (entry.type !== 'http') throw new Error('expected http entry')
    const names = entry.headers.map((h) => h.name)
    expect(names).not.toContain('X-BrowserOS-Default-Window-Id')
  })

  it('writes the window-id header when defaultWindowId is set', () => {
    const entry = buildBrowserOsSelfMcpEntry({
      serverPort: 9100,
      conversationId: 'conv-1',
      providerId: 'claude-code',
      defaultWindowId: 7,
    })
    if (entry.type !== 'http') throw new Error('expected http entry')
    const header = (name: string) =>
      entry.headers.find((h) => h.name === name)?.value
    expect(header('X-BrowserOS-Default-Window-Id')).toBe('7')
  })

  it('writes the tab-group header only when defaultTabGroupId is a non-empty string', () => {
    const withGroup = buildBrowserOsSelfMcpEntry({
      serverPort: 9100,
      conversationId: 'conv-1',
      providerId: 'claude-code',
      defaultTabGroupId: 'group-abc',
    })
    const withoutGroup = buildBrowserOsSelfMcpEntry({
      serverPort: 9100,
      conversationId: 'conv-1',
      providerId: 'claude-code',
      defaultTabGroupId: '',
    })
    if (withGroup.type !== 'http' || withoutGroup.type !== 'http') {
      throw new Error('expected http entries')
    }
    expect(
      withGroup.headers.find(
        (h) => h.name === 'X-BrowserOS-Default-Tab-Group-Id',
      )?.value,
    ).toBe('group-abc')
    expect(
      withoutGroup.headers.find(
        (h) => h.name === 'X-BrowserOS-Default-Tab-Group-Id',
      ),
    ).toBeUndefined()
  })

  it('accepts windowId 0 as a valid integer header value', () => {
    const entry = buildBrowserOsSelfMcpEntry({
      serverPort: 9100,
      conversationId: 'conv-1',
      providerId: 'claude-code',
      defaultWindowId: 0,
    })
    if (entry.type !== 'http') throw new Error('expected http entry')
    expect(
      entry.headers.find((h) => h.name === 'X-BrowserOS-Default-Window-Id')
        ?.value,
    ).toBe('0')
  })
})
