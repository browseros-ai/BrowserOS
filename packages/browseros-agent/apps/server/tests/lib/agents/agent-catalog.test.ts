/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  AGENT_ADAPTER_CATALOG,
  getAgentAdapterDescriptor,
  isSupportedAgentModel,
  isSupportedReasoningEffort,
} from '../../../src/lib/agents/agent-catalog'

describe('AGENT_ADAPTER_CATALOG', () => {
  it('exposes Claude, Codex, and OpenClaw adapters with model and effort options', () => {
    expect(AGENT_ADAPTER_CATALOG.map((adapter) => adapter.id)).toEqual([
      'claude',
      'codex',
      'openclaw',
    ])

    expect(getAgentAdapterDescriptor('claude')).toMatchObject({
      id: 'claude',
      name: 'Claude Code',
      defaultModelId: 'haiku',
      defaultReasoningEffort: 'medium',
      modelControl: 'best-effort',
    })

    expect(getAgentAdapterDescriptor('codex')).toMatchObject({
      id: 'codex',
      name: 'Codex',
      defaultModelId: 'gpt-5.5',
      defaultReasoningEffort: 'medium',
      modelControl: 'best-effort',
    })

    expect(getAgentAdapterDescriptor('openclaw')).toMatchObject({
      id: 'openclaw',
      name: 'OpenClaw',
      defaultModelId: 'default',
      defaultReasoningEffort: 'medium',
      modelControl: 'best-effort',
    })
    // OpenClaw has no per-session model picker; the model lives in the
    // gateway-side agent record and is sourced from the LlmProviderConfig.
    expect(getAgentAdapterDescriptor('openclaw')?.models).toEqual([])

    expect(isSupportedAgentModel('claude', 'haiku')).toBe(true)
    expect(isSupportedAgentModel('codex', 'gpt-5.5')).toBe(true)
    // Empty models list → all model ids are accepted ("default" passthrough).
    expect(isSupportedAgentModel('openclaw', undefined)).toBe(true)
    expect(isSupportedAgentModel('openclaw', 'default')).toBe(true)
    expect(isSupportedAgentModel('openclaw', 'gpt-5.5')).toBe(false)

    expect(isSupportedReasoningEffort('codex', 'xhigh')).toBe(true)
    expect(isSupportedReasoningEffort('claude', 'banana')).toBe(false)
    expect(isSupportedReasoningEffort('openclaw', 'adaptive')).toBe(true)
    expect(isSupportedReasoningEffort('openclaw', 'xhigh')).toBe(false)
  })
})
