import { describe, expect, test } from 'bun:test'

import {
  AgentNotInstalledError,
  McpManagerError,
  UnsupportedTransportError,
} from '../../src/errors'

describe('UnsupportedTransportError', () => {
  test('extends McpManagerError so instanceof checks compose', () => {
    const err = new UnsupportedTransportError('claude-desktop', 'http', {
      supported: ['stdio'],
      hint: 'wrap with npx -y mcp-remote',
    })
    expect(err).toBeInstanceOf(McpManagerError)
    expect(err).toBeInstanceOf(UnsupportedTransportError)
    expect(err.name).toBe('UnsupportedTransportError')
  })

  test('exposes agent, transport, and supported list', () => {
    const err = new UnsupportedTransportError('codex', 'sse', {
      supported: ['stdio'],
      hint: 'use the mcp-remote shim',
    })
    expect(err.agent).toBe('codex')
    expect(err.transport).toBe('sse')
    expect(err.details.supported).toEqual(['stdio'])
    expect(err.details.hint).toContain('mcp-remote')
  })

  test('message names the requested transport and the supported set', () => {
    const err = new UnsupportedTransportError('claude-desktop', 'http', {
      supported: ['stdio'],
      hint: 'see README',
    })
    expect(err.message).toContain('claude-desktop')
    expect(err.message).toContain('"http"')
    expect(err.message).toContain('supported: stdio')
  })
})

describe('AgentNotInstalledError', () => {
  test('extends McpManagerError so instanceof checks compose', () => {
    const err = new AgentNotInstalledError(
      'cursor',
      '/home/dev/.cursor/mcp.json',
      '/home/dev/.cursor',
    )
    expect(err).toBeInstanceOf(McpManagerError)
    expect(err.name).toBe('AgentNotInstalledError')
    expect(err.agent).toBe('cursor')
    expect(err.configPath).toBe('/home/dev/.cursor/mcp.json')
    expect(err.parentDir).toBe('/home/dev/.cursor')
  })

  test('message includes the agent id, config path, and parent dir', () => {
    const err = new AgentNotInstalledError(
      'gemini',
      '/tmp/x/.gemini/settings.json',
      '/tmp/x/.gemini',
    )
    expect(err.message).toContain('gemini')
    expect(err.message).toContain('/tmp/x/.gemini/settings.json')
    expect(err.message).toContain('/tmp/x/.gemini')
  })

  test('message contains no em-dashes (repo writing rule)', () => {
    const err = new AgentNotInstalledError('zed', '/a/b', '/a')
    expect(err.message.includes('—')).toBe(false)
  })
})
