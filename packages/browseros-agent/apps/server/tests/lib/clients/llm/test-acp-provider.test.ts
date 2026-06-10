/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'

let nextProbeResult: Record<string, unknown> | null = null
let lastProbeInput: Record<string, unknown> | null = null

mock.module('../../../../src/api/services/acpx-probe/probeAgent', () => ({
  probeAcpAgent: async (input: Record<string, unknown>) => {
    lastProbeInput = input
    return nextProbeResult
  },
}))

const { testAcpProvider } = await import(
  '../../../../src/lib/clients/llm/test-acp-provider'
)

beforeEach(() => {
  nextProbeResult = null
  lastProbeInput = null
})

function probeOK(overrides: Record<string, unknown> = {}) {
  return {
    models: [{ id: 'sonnet' }, { id: 'haiku' }],
    reasoning: { values: ['low', 'medium'], defaultValue: 'medium' },
    supportsConfigOption: true,
    agentInfo: { name: 'claude', title: 'Claude Code' },
    protocolVersion: 1,
    ...overrides,
  }
}

describe('testAcpProvider — happy path', () => {
  it('returns success when the agent advertises the requested model', async () => {
    nextProbeResult = probeOK()
    const result = await testAcpProvider({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(true)
    expect(result.message).toContain('Claude Code')
    expect(result.message).toContain('2 model(s)')
    expect(result.responseTime).toBeDefined()
  })

  it('resolves the built-in agent id from the provider type', async () => {
    nextProbeResult = probeOK()
    await testAcpProvider({ provider: 'claude-code', model: 'sonnet' })
    expect(lastProbeInput?.agentId).toBe('claude')

    nextProbeResult = probeOK()
    await testAcpProvider({ provider: 'codex', model: 'sonnet' })
    expect(lastProbeInput?.agentId).toBe('codex')
  })

  it('honours an explicit acpAgentId override', async () => {
    nextProbeResult = probeOK()
    await testAcpProvider({
      provider: 'claude-code',
      model: 'sonnet',
      acpAgentId: 'claude-experimental',
    })
    expect(lastProbeInput?.agentId).toBe('claude-experimental')
  })

  it('forwards acpCommand and cwd for acp-custom', async () => {
    nextProbeResult = probeOK()
    await testAcpProvider({
      provider: 'acp-custom',
      model: 'sonnet',
      acpAgentId: 'my-agent',
      acpCommand: 'my-bin acp',
      acpFixedWorkspacePath: '/tmp/x',
    })
    expect(lastProbeInput?.agentId).toBe('my-agent')
    expect(lastProbeInput?.command).toBe('my-bin acp')
    expect(lastProbeInput?.cwd).toBe('/tmp/x')
  })
})

describe('testAcpProvider — failure modes', () => {
  it('reports the agent_crashed code with a human-readable message', async () => {
    nextProbeResult = probeOK({
      error: { code: 'agent_crashed', message: 'died' },
    })
    const result = await testAcpProvider({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('crashed')
  })

  it('reports spawn_failed with a recovery hint', async () => {
    nextProbeResult = probeOK({
      error: { code: 'spawn_failed', message: 'no such file' },
    })
    const result = await testAcpProvider({
      provider: 'codex',
      model: 'gpt-5.5',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('PATH')
  })

  it('reports auth_required with a sign-in hint', async () => {
    nextProbeResult = probeOK({
      error: { code: 'auth_required', message: 'no creds' },
    })
    const result = await testAcpProvider({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('Sign in')
  })

  it('falls through to the raw message for unknown codes', async () => {
    nextProbeResult = probeOK({
      error: { code: 'something_new', message: 'undefined behaviour' },
    })
    const result = await testAcpProvider({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('undefined behaviour')
  })

  it('fails when the probe returns zero models', async () => {
    nextProbeResult = probeOK({ models: [] })
    const result = await testAcpProvider({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('did not advertise')
  })

  it('fails when the requested model is not advertised', async () => {
    nextProbeResult = probeOK({ models: [{ id: 'opus' }] })
    const result = await testAcpProvider({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('sonnet')
    expect(result.message).toContain('Available: opus')
  })

  it('rejects acp-custom when no agentId is provided', async () => {
    const result = await testAcpProvider({
      provider: 'acp-custom',
      model: 'sonnet',
      acpCommand: 'my-bin acp',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('agent id')
  })
})
