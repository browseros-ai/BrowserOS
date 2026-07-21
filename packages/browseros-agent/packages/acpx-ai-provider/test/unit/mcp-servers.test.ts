import { describe, expect, test } from 'bun:test'
import { toRuntimeMcpServers } from '../../src/mcp-servers.ts'
import type { AcpxMcpServerConfig } from '../../src/types.ts'

describe('toRuntimeMcpServers', () => {
  test('returns undefined when input is undefined', () => {
    expect(toRuntimeMcpServers(undefined)).toBeUndefined()
  })

  test('returns empty array for empty input', () => {
    expect(toRuntimeMcpServers([])).toEqual([])
  })

  test('converts stdio env record to ACP EnvVariable array', () => {
    const input: AcpxMcpServerConfig[] = [
      {
        type: 'stdio',
        name: 'fs',
        command: 'mcp-server-filesystem',
        args: ['/tmp'],
        env: { FOO: 'bar', LOG_LEVEL: 'info' },
      },
    ]
    const [out] = toRuntimeMcpServers(input) ?? []
    expect(out).toEqual({
      name: 'fs',
      command: 'mcp-server-filesystem',
      args: ['/tmp'],
      env: [
        { name: 'FOO', value: 'bar' },
        { name: 'LOG_LEVEL', value: 'info' },
      ],
    } as typeof out)
  })

  test('defaults stdio env and args to empty arrays when omitted', () => {
    const [out] =
      toRuntimeMcpServers([{ type: 'stdio', name: 'fs', command: 'mcp' }]) ?? []
    expect(out).toEqual({
      name: 'fs',
      command: 'mcp',
      args: [],
      env: [],
    } as typeof out)
  })

  test('converts http headers record to ACP HttpHeader array', () => {
    const [out] =
      toRuntimeMcpServers([
        {
          type: 'http',
          name: 'remote',
          url: 'https://example.com/mcp',
          headers: { Authorization: 'Bearer token', 'X-Foo': 'bar' },
        },
      ]) ?? []
    expect(out).toEqual({
      type: 'http',
      name: 'remote',
      url: 'https://example.com/mcp',
      headers: [
        { name: 'Authorization', value: 'Bearer token' },
        { name: 'X-Foo', value: 'bar' },
      ],
    } as typeof out)
  })

  test('preserves sse transport type', () => {
    const [out] =
      toRuntimeMcpServers([
        {
          type: 'sse',
          name: 'stream',
          url: 'https://example.com/sse',
        },
      ]) ?? []
    expect(out).toEqual({
      type: 'sse',
      name: 'stream',
      url: 'https://example.com/sse',
      headers: [],
    } as typeof out)
  })

  test('handles mixed transports in one call', () => {
    const out = toRuntimeMcpServers([
      { type: 'stdio', name: 'a', command: 'cmd' },
      { type: 'http', name: 'b', url: 'https://x' },
    ])
    expect(out).toHaveLength(2)
    expect(out?.[0]).toHaveProperty('command', 'cmd')
    expect(out?.[1]).toHaveProperty('url', 'https://x')
  })
})
