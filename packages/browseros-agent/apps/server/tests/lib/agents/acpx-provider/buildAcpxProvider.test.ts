/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'

// Hold the most recent call so each test asserts in isolation.
let lastSettings: Record<string, unknown> | null = null
const fakeProvider = { languageModel: () => ({ kind: 'fake' }) }

mock.module('acpx-ai-provider', () => ({
  createAcpxProvider: (settings: Record<string, unknown>) => {
    lastSettings = settings
    return fakeProvider
  },
}))

mock.module('../../../../src/lib/browseros-dir', () => ({
  getBrowserosDir: () => '/tmp/browseros-test',
}))

const mod = await import(
  '../../../../src/lib/agents/acpx-provider/buildAcpxProvider'
)
const { buildAcpxProvider, __internal__ } = mod

beforeEach(() => {
  lastSettings = null
})

describe('buildAcpxProvider — defaults', () => {
  it('forwards the right defaults to createAcpxProvider', async () => {
    const provider = await buildAcpxProvider({
      conversationId: 'conv-1',
      agentId: 'claude',
    })
    expect(provider).toBe(fakeProvider)
    expect(lastSettings).toEqual({
      agent: 'claude',
      cwd: expect.any(String),
      sessionKey: 'conv-1',
      sessionMode: 'persistent',
      stateDir: '/tmp/browseros-test/acpx-state',
      resumeSessionId: undefined,
      agentRegistryOverrides: {},
      permissionMode: 'approve-reads',
      nonInteractivePermissions: 'deny',
      onPermissionRequest: undefined,
      mcpServers: undefined,
    })
  })

  it('uses $HOME for the working directory when no path is supplied', async () => {
    await buildAcpxProvider({ conversationId: 'conv-2', agentId: 'codex' })
    // homedir is platform-dependent but always non-empty and absolute
    expect(typeof lastSettings?.cwd).toBe('string')
    expect((lastSettings?.cwd as string).length).toBeGreaterThan(0)
  })

  it('honours workspacePath when provided', async () => {
    await buildAcpxProvider({
      conversationId: 'conv-3',
      agentId: 'claude',
      workspacePath: '/tmp/workspace-a',
    })
    expect(lastSettings?.cwd).toBe('/tmp/workspace-a')
  })

  it('uses the provided sessionKey when it differs from conversationId', async () => {
    await buildAcpxProvider({
      conversationId: 'conv-4',
      agentId: 'claude',
      sessionKey: 'session-custom',
    })
    expect(lastSettings?.sessionKey).toBe('session-custom')
  })

  it('honours an explicit stateDir override', async () => {
    await buildAcpxProvider({
      conversationId: 'conv-5',
      agentId: 'claude',
      stateDir: '/tmp/explicit-state',
    })
    expect(lastSettings?.stateDir).toBe('/tmp/explicit-state')
  })
})

describe('buildAcpxProvider — agentRegistryOverrides', () => {
  it('passes overrides verbatim to createAcpxProvider', async () => {
    const overrides = { hermes: 'hermes acp', custom: 'my-bin acp' }
    await buildAcpxProvider({
      conversationId: 'conv-6',
      agentId: 'custom',
      agentRegistryOverrides: overrides,
    })
    expect(lastSettings?.agentRegistryOverrides).toEqual(overrides)
  })

  it('defaults to an empty overrides record', async () => {
    await buildAcpxProvider({ conversationId: 'conv-7', agentId: 'claude' })
    expect(lastSettings?.agentRegistryOverrides).toEqual({})
  })
})

describe('buildAcpxProvider — mcpServers translation', () => {
  it('converts stdio entries to Record env shape', async () => {
    await buildAcpxProvider({
      conversationId: 'conv-8',
      agentId: 'claude',
      mcpServers: [
        {
          type: 'stdio',
          name: 'fs',
          command: 'mcp-fs',
          args: ['--read-only'],
          env: [
            { name: 'FS_ROOT', value: '/tmp/x' },
            { name: 'FS_VERBOSE', value: '1' },
          ],
        },
      ],
    })
    expect(lastSettings?.mcpServers).toEqual([
      {
        type: 'stdio',
        name: 'fs',
        command: 'mcp-fs',
        args: ['--read-only'],
        env: { FS_ROOT: '/tmp/x', FS_VERBOSE: '1' },
      },
    ])
  })

  it('converts http / sse entries to Record headers shape', async () => {
    await buildAcpxProvider({
      conversationId: 'conv-9',
      agentId: 'claude',
      mcpServers: [
        {
          type: 'http',
          name: 'remote',
          url: 'https://mcp.example.com/mcp',
          headers: [
            { name: 'Authorization', value: 'Bearer secret' },
            { name: 'X-Project', value: '1' },
          ],
        },
        {
          type: 'sse',
          name: 'streaming',
          url: 'https://stream.example.com/sse',
          headers: [],
        },
      ],
    })
    expect(lastSettings?.mcpServers).toEqual([
      {
        type: 'http',
        name: 'remote',
        url: 'https://mcp.example.com/mcp',
        headers: { Authorization: 'Bearer secret', 'X-Project': '1' },
      },
      {
        type: 'sse',
        name: 'streaming',
        url: 'https://stream.example.com/sse',
        headers: {},
      },
    ])
  })

  it('leaves mcpServers undefined when caller passes nothing', async () => {
    await buildAcpxProvider({ conversationId: 'conv-10', agentId: 'claude' })
    expect(lastSettings?.mcpServers).toBeUndefined()
  })
})

describe('buildAcpxProvider — permission wiring', () => {
  it('forwards a custom onPermissionRequest callback', async () => {
    const handler = async () => undefined
    await buildAcpxProvider({
      conversationId: 'conv-11',
      agentId: 'claude',
      onPermissionRequest: handler,
    })
    expect(lastSettings?.onPermissionRequest).toBe(handler)
  })

  it('respects an explicit permissionMode + nonInteractivePermissions override', async () => {
    await buildAcpxProvider({
      conversationId: 'conv-12',
      agentId: 'claude',
      permissionMode: 'allow-all',
      nonInteractivePermissions: 'allow',
    })
    expect(lastSettings?.permissionMode).toBe('allow-all')
    expect(lastSettings?.nonInteractivePermissions).toBe('allow')
  })
})

describe('__internal__ helpers', () => {
  it('toProviderShape passes through stdio fields when env is empty', () => {
    const result = __internal__.toProviderShape({
      type: 'stdio',
      name: 'noop',
      command: 'true',
      args: [],
      env: [],
    })
    expect(result).toEqual({
      type: 'stdio',
      name: 'noop',
      command: 'true',
      args: [],
      env: {},
    })
  })

  it('pairsToRecord keeps the last value when names collide', () => {
    const result = __internal__.pairsToRecord([
      { name: 'X', value: 'first' },
      { name: 'X', value: 'second' },
    ])
    expect(result).toEqual({ X: 'second' })
  })
})
