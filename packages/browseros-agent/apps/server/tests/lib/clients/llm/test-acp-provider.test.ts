/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, expect, it } from 'bun:test'
import type {
  ServerAcpxProbeInput,
  ServerAcpxProbeResult,
} from '../../../../src/api/services/acpx-probe/probeAgent'
import {
  type TestAcpProviderInput,
  testAcpProvider,
} from '../../../../src/lib/clients/llm/test-acp-provider'

// Injection beats mock.module here: the sibling probeAgent.test.ts wants the
// real probeAcpAgent, and bun's mock.module leaks across files in the same
// test process. Threading the stub through testAcpProvider keeps both isolated.
let nextProbeResult: ServerAcpxProbeResult | null = null
let lastProbeInput: ServerAcpxProbeInput | null = null

const probeStub = async (
  input: ServerAcpxProbeInput,
): Promise<ServerAcpxProbeResult> => {
  lastProbeInput = input
  if (!nextProbeResult) throw new Error('probe stub: no nextProbeResult set')
  return nextProbeResult
}

const runTest = (input: TestAcpProviderInput) =>
  testAcpProvider(input, { probe: probeStub })

beforeEach(() => {
  nextProbeResult = null
  lastProbeInput = null
})

function probeOK(
  overrides: Partial<ServerAcpxProbeResult> = {},
): ServerAcpxProbeResult {
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
    const result = await runTest({
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
    await runTest({ provider: 'claude-code', model: 'sonnet' })
    expect(lastProbeInput?.agentId).toBe('claude')

    nextProbeResult = probeOK()
    await runTest({ provider: 'codex', model: 'sonnet' })
    expect(lastProbeInput?.agentId).toBe('codex')
  })

  it('honours an explicit acpAgentId override', async () => {
    nextProbeResult = probeOK()
    await runTest({
      provider: 'claude-code',
      model: 'sonnet',
      acpAgentId: 'claude-experimental',
    })
    expect(lastProbeInput?.agentId).toBe('claude-experimental')
  })

  it('forwards acpCommand and cwd for acp-custom', async () => {
    nextProbeResult = probeOK()
    await runTest({
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
    const result = await runTest({
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
    const result = await runTest({
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
    const result = await runTest({
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
    const result = await runTest({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('undefined behaviour')
  })

  it('fails when the probe returns zero models', async () => {
    nextProbeResult = probeOK({ models: [] })
    const result = await runTest({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('did not advertise')
  })

  it('fails when the requested model is not advertised', async () => {
    nextProbeResult = probeOK({ models: [{ id: 'opus' }] })
    const result = await runTest({
      provider: 'claude-code',
      model: 'sonnet',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('sonnet')
    expect(result.message).toContain('Available: opus')
  })

  it('rejects acp-custom when no agentId is provided', async () => {
    const result = await runTest({
      provider: 'acp-custom',
      model: 'sonnet',
      acpCommand: 'my-bin acp',
    })
    expect(result.success).toBe(false)
    expect(result.message).toContain('agent id')
  })
})
