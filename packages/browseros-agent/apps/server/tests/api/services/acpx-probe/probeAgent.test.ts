/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'

interface CapturedCall {
  agent?: string
  command?: string
  cwd?: string
  authPolicy?: string
  timeoutMs?: number
}

let lastCall: CapturedCall | null = null
let nextResult: unknown = null

mock.module('acp-probe', () => ({
  probeAgent: async (input: CapturedCall) => {
    lastCall = input
    return nextResult
  },
}))

const mod = await import('../../../../src/api/services/acpx-probe/probeAgent')
const { probeAcpAgent } = mod

beforeEach(() => {
  lastCall = null
  nextResult = null
  delete process.env.BROWSEROS_ACPX_PROBE_TIMEOUT_MS
})

function baseProbeResult(overrides: Record<string, unknown> = {}) {
  return {
    agent: {
      id: 'claude',
      command: 'claude',
      argv: ['claude'],
      probedAt: '',
      durationMs: 1,
    },
    protocolVersion: 1,
    agentInfo: { name: 'claude', title: 'Claude Code', version: '0.31.4' },
    capabilities: {},
    authMethods: [],
    models: [
      { id: 'sonnet', name: 'Sonnet' },
      { id: 'haiku', name: 'Haiku' },
    ],
    modes: [],
    configOptions: [],
    reasoning: {
      configId: 'effort',
      values: ['low', 'medium', 'high'],
      defaultValue: 'medium',
    },
    modelConfig: {
      configId: 'model',
      values: ['sonnet', 'haiku'],
      currentValue: 'sonnet',
    },
    supportsConfigOption: true,
    raw: { initialize: {}, newSession: null },
    ...overrides,
  }
}

describe('probeAcpAgent — input shape', () => {
  it('rejects when neither agentId nor command is provided', async () => {
    await expect(probeAcpAgent({})).rejects.toThrow(
      'Either agentId or command is required',
    )
  })

  it('forwards agentId to the underlying probe', async () => {
    nextResult = baseProbeResult()
    await probeAcpAgent({ agentId: 'claude' })
    expect(lastCall?.agent).toBe('claude')
    expect(lastCall?.authPolicy).toBe('skip')
  })

  it('forwards command and cwd for acp-custom', async () => {
    nextResult = baseProbeResult()
    await probeAcpAgent({ command: 'my-bin acp', cwd: '/tmp/x' })
    expect(lastCall?.command).toBe('my-bin acp')
    expect(lastCall?.cwd).toBe('/tmp/x')
  })

  it('defaults the timeout to 10 seconds', async () => {
    nextResult = baseProbeResult()
    await probeAcpAgent({ agentId: 'claude' })
    expect(lastCall?.timeoutMs).toBe(10_000)
  })

  it('honours an explicit timeoutMs', async () => {
    nextResult = baseProbeResult()
    await probeAcpAgent({ agentId: 'claude', timeoutMs: 5_000 })
    expect(lastCall?.timeoutMs).toBe(5_000)
  })

  it('honours BROWSEROS_ACPX_PROBE_TIMEOUT_MS when in the [1000, 60000] range', async () => {
    process.env.BROWSEROS_ACPX_PROBE_TIMEOUT_MS = '20000'
    nextResult = baseProbeResult()
    await probeAcpAgent({ agentId: 'claude' })
    expect(lastCall?.timeoutMs).toBe(20_000)
  })

  it('ignores BROWSEROS_ACPX_PROBE_TIMEOUT_MS when out of range', async () => {
    process.env.BROWSEROS_ACPX_PROBE_TIMEOUT_MS = '999'
    nextResult = baseProbeResult()
    await probeAcpAgent({ agentId: 'claude' })
    expect(lastCall?.timeoutMs).toBe(10_000)
  })
})

describe('probeAcpAgent — normalisation', () => {
  it('filters advertised models down to modelConfig.values when present', async () => {
    nextResult = baseProbeResult({
      models: [
        { id: 'sonnet', name: 'Sonnet' },
        { id: 'haiku', name: 'Haiku' },
        { id: 'sonnet/high', name: 'Sonnet (high)' },
      ],
      modelConfig: { configId: 'model', values: ['sonnet', 'haiku'] },
    })
    const out = await probeAcpAgent({ agentId: 'claude' })
    expect(out.models.map((m) => m.id)).toEqual(['sonnet', 'haiku'])
  })

  it('falls back to advertised models when modelConfig is null', async () => {
    nextResult = baseProbeResult({
      modelConfig: null,
      models: [
        { id: 'a', name: 'A' },
        { id: 'b', name: 'B' },
      ],
    })
    const out = await probeAcpAgent({ agentId: 'codex' })
    expect(out.models.map((m) => m.id)).toEqual(['a', 'b'])
  })

  it('forwards reasoning values and defaultValue', async () => {
    nextResult = baseProbeResult({
      reasoning: {
        configId: 'effort',
        values: ['low', 'medium', 'high', 'xhigh', 'max'],
        defaultValue: 'high',
      },
    })
    const out = await probeAcpAgent({ agentId: 'claude' })
    expect(out.reasoning?.values).toEqual([
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ])
    expect(out.reasoning?.defaultValue).toBe('high')
  })

  it('returns null reasoning when the agent has no thought_level config', async () => {
    nextResult = baseProbeResult({ reasoning: null })
    const out = await probeAcpAgent({ agentId: 'gemini' })
    expect(out.reasoning).toBeNull()
  })

  it('passes through agentInfo, supportsConfigOption, protocolVersion', async () => {
    nextResult = baseProbeResult({
      agentInfo: { name: 'codex', title: 'Codex CLI', version: '0.12.0' },
      supportsConfigOption: false,
      protocolVersion: 2,
    })
    const out = await probeAcpAgent({ agentId: 'codex' })
    expect(out.agentInfo).toEqual({
      name: 'codex',
      title: 'Codex CLI',
      version: '0.12.0',
    })
    expect(out.supportsConfigOption).toBe(false)
    expect(out.protocolVersion).toBe(2)
  })

  it('surfaces probe errors instead of throwing', async () => {
    nextResult = baseProbeResult({
      error: {
        code: 'auth_required',
        message: 'Agent declined session/new without credentials',
        acpError: { code: -32603, message: 'auth required' },
      },
    })
    const out = await probeAcpAgent({ agentId: 'claude' })
    expect(out.error?.code).toBe('auth_required')
    expect(out.error?.acpErrorCode).toBe(-32603)
  })
})
