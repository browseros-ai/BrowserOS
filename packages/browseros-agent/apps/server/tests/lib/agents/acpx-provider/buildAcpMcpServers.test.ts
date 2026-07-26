/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { buildAcpMcpServers } from '../../../../src/lib/agents/acpx-provider/buildAcpMcpServers'

const baseOpts = {
  serverPort: 9100,
  conversationId: 'conv-1',
  providerId: 'claude-code',
}

describe('buildAcpMcpServers', () => {
  it('always prepends the BrowserOS self entry as the first element', () => {
    const out = buildAcpMcpServers(baseOpts)
    expect(out).toHaveLength(1)
    expect(out[0]?.name).toBe('browseros')
  })

  it('appends user-configured custom MCP servers after BrowserOS', () => {
    const out = buildAcpMcpServers({
      ...baseOpts,
      customMcpServers: [
        { name: 'github', url: 'https://mcp.example.com/github' },
        { name: 'figma', url: 'https://mcp.example.com/figma' },
      ],
    })
    expect(out.map((s) => s.name)).toEqual(['browseros', 'github', 'figma'])
    expect(out[1]?.type).toBe('http')
    if (out[1]?.type === 'http') {
      expect(out[1].url).toBe('https://mcp.example.com/github')
      expect(out[1].headers).toEqual([])
    }
  })

  it('keeps BrowserOS first even when a custom server shares the name', () => {
    const out = buildAcpMcpServers({
      ...baseOpts,
      customMcpServers: [
        { name: 'browseros', url: 'https://impostor.example.com/mcp' },
      ],
    })
    expect(out[0]?.name).toBe('browseros')
    if (out[0]?.type === 'http') {
      expect(out[0].url).toBe('http://127.0.0.1:9100/mcp')
    }
    expect(out[1]?.name).toBe('browseros')
    if (out[1]?.type === 'http') {
      expect(out[1].url).toBe('https://impostor.example.com/mcp')
    }
  })

  it('forwards defaultWindowId into the BrowserOS entry headers', () => {
    const out = buildAcpMcpServers({ ...baseOpts, defaultWindowId: 11 })
    if (out[0]?.type !== 'http') throw new Error('expected http entry')
    expect(
      out[0].headers.find((h) => h.name === 'X-BrowserOS-Default-Window-Id')
        ?.value,
    ).toBe('11')
  })

  it('forwards enabled managed server names into the BrowserOS entry headers', () => {
    const out = buildAcpMcpServers({
      ...baseOpts,
      enabledMcpServers: ['Slack', 'Google Docs'],
    })
    if (out[0]?.type !== 'http') throw new Error('expected http entry')
    expect(
      out[0].headers.find((h) => h.name === 'X-BrowserOS-Managed-Mcp-Servers')
        ?.value,
    ).toBe('Slack,Google%20Docs')
  })

  it('handles an undefined customMcpServers list', () => {
    const out = buildAcpMcpServers(baseOpts)
    expect(out).toHaveLength(1)
  })
})
