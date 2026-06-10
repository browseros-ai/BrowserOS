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
  it('uses the configOptions[id=model] picker as the model source when present', async () => {
    nextResult = baseProbeResult({
      models: [
        { id: 'sonnet', name: 'Sonnet (display)' },
        { id: 'haiku', name: 'Haiku (display)' },
      ],
      configOptions: [
        {
          id: 'model',
          name: 'Model',
          type: 'select',
          currentValue: 'sonnet',
          options: [
            { value: 'sonnet', name: 'Sonnet', description: 'Everyday' },
            { value: 'haiku', name: 'Haiku', description: 'Fast' },
          ],
        },
      ],
      modelConfig: { configId: 'model', values: ['sonnet', 'haiku'] },
    })
    const out = await probeAcpAgent({ agentId: 'claude' })
    expect(out.models).toEqual([
      { id: 'sonnet', name: 'Sonnet', description: 'Everyday' },
      { id: 'haiku', name: 'Haiku', description: 'Fast' },
    ])
  })

  it('returns the bare codex picker ids even when advertised models are compound <model>/<effort>', async () => {
    nextResult = baseProbeResult({
      models: [
        { id: 'gpt-5.5/low', name: 'GPT-5.5 (low)' },
        { id: 'gpt-5.5/medium', name: 'GPT-5.5 (medium)' },
        { id: 'gpt-5.3-codex/low', name: 'gpt-5.3-codex (low)' },
      ],
      configOptions: [
        {
          id: 'model',
          name: 'Model',
          type: 'select',
          currentValue: 'gpt-5.5',
          options: [
            { value: 'gpt-5.5', name: 'GPT-5.5' },
            { value: 'gpt-5.3-codex', name: 'gpt-5.3-codex' },
          ],
        },
      ],
      modelConfig: {
        configId: 'model',
        values: ['gpt-5.5', 'gpt-5.3-codex'],
        currentValue: 'gpt-5.5',
      },
    })
    const out = await probeAcpAgent({ agentId: 'codex' })
    expect(out.models.map((m) => m.id)).toEqual(['gpt-5.5', 'gpt-5.3-codex'])
  })

  it('falls back to advertised models when no configOptions[id=model] picker exists', async () => {
    nextResult = baseProbeResult({
      configOptions: [],
      modelConfig: null,
      reasoning: null,
      models: [
        { id: 'a', name: 'A' },
        { id: 'b', name: 'B' },
      ],
    })
    const out = await probeAcpAgent({ agentId: 'gemini' })
    expect(out.models.map((m) => m.id)).toEqual(['a', 'b'])
    expect(out.reasoning).toBeNull()
  })

  it('splits compound `model[effort]` ids into bare models + effort list when no picker exists', async () => {
    // Mirrors the actual codex-acp response when supportsConfigOption=false:
    // no configOptions, reasoning=null, models carry effort in the id.
    nextResult = baseProbeResult({
      configOptions: [],
      modelConfig: null,
      reasoning: null,
      supportsConfigOption: false,
      models: [
        {
          id: 'gpt-5.3-codex[low]',
          name: 'gpt-5.3-codex (low)',
          description:
            'Coding-optimized model. Fast responses with lighter reasoning',
        },
        {
          id: 'gpt-5.3-codex[medium]',
          name: 'gpt-5.3-codex (medium)',
          description:
            'Coding-optimized model. Balances speed and reasoning depth for everyday tasks',
        },
        {
          id: 'gpt-5.5[low]',
          name: 'GPT-5.5 (low)',
          description:
            'Frontier model for complex coding, research, and real-world work. Fast responses with lighter reasoning',
        },
        {
          id: 'gpt-5.5[xhigh]',
          name: 'GPT-5.5 (xhigh)',
          description:
            'Frontier model for complex coding, research, and real-world work. Extra high reasoning depth for complex problems',
        },
      ],
    })
    const out = await probeAcpAgent({ agentId: 'codex' })
    expect(out.models).toEqual([
      {
        id: 'gpt-5.3-codex',
        name: 'gpt-5.3-codex',
        description: 'Coding-optimized model.',
      },
      {
        id: 'gpt-5.5',
        name: 'GPT-5.5',
        description:
          'Frontier model for complex coding, research, and real-world work.',
      },
    ])
    expect(out.reasoning).toEqual({
      values: ['low', 'medium', 'xhigh'],
      defaultValue: 'medium',
    })
  })

  it('handles the documented `model/effort` slash form as well', async () => {
    nextResult = baseProbeResult({
      configOptions: [],
      modelConfig: null,
      reasoning: null,
      supportsConfigOption: false,
      models: [
        { id: 'gpt-5.5/low', name: 'GPT-5.5 (low)' },
        { id: 'gpt-5.5/medium', name: 'GPT-5.5 (medium)' },
      ],
    })
    const out = await probeAcpAgent({ agentId: 'codex' })
    expect(out.models.map((m) => m.id)).toEqual(['gpt-5.5'])
    expect(out.reasoning?.values).toEqual(['low', 'medium'])
  })

  it('falls back to medium-or-first when there is no obvious default effort', async () => {
    nextResult = baseProbeResult({
      configOptions: [],
      modelConfig: null,
      reasoning: null,
      supportsConfigOption: false,
      models: [
        { id: 'foo[low]', name: 'foo (low)' },
        { id: 'foo[high]', name: 'foo (high)' },
      ],
    })
    const out = await probeAcpAgent({ agentId: 'codex' })
    expect(out.reasoning?.defaultValue).toBe('low')
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
