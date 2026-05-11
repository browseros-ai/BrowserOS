/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  AGENT_ADAPTER_CATALOG,
  getAgentAdapterDescriptor,
  isAgentAdapter,
  isSupportedAgentModel,
  isSupportedReasoningEffort,
} from '../../../src/lib/agents/agent-catalog'

describe('AGENT_ADAPTER_CATALOG', () => {
  it('exposes Claude, Codex, OpenClaw, Hermes, and Custom adapters with model and effort options', () => {
    expect(AGENT_ADAPTER_CATALOG.map((adapter) => adapter.id)).toEqual([
      'claude',
      'codex',
      'openclaw',
      'hermes',
      'custom',
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
    expect(isSupportedAgentModel('claude', 'claude-opus-4-7')).toBe(true)
    expect(isSupportedAgentModel('claude', 'claude-sonnet-4-6')).toBe(true)
    expect(isSupportedAgentModel('claude', 'claude-haiku-4-5')).toBe(true)
    expect(isSupportedAgentModel('claude', 'claude-not-real')).toBe(false)
    expect(isSupportedAgentModel('codex', 'gpt-5.5')).toBe(true)
    expect(isSupportedAgentModel('codex', 'gpt-5.4-mini')).toBe(true)
    expect(isSupportedAgentModel('codex', 'codex-auto-review')).toBe(false)
    // Empty models list → all model ids are accepted ("default" passthrough).
    expect(isSupportedAgentModel('openclaw', undefined)).toBe(true)
    expect(isSupportedAgentModel('openclaw', 'default')).toBe(true)
    expect(isSupportedAgentModel('openclaw', 'gpt-5.5')).toBe(false)

    expect(isSupportedReasoningEffort('codex', 'xhigh')).toBe(true)
    expect(isSupportedReasoningEffort('claude', 'banana')).toBe(false)
    expect(isSupportedReasoningEffort('openclaw', 'adaptive')).toBe(true)
    expect(isSupportedReasoningEffort('openclaw', 'xhigh')).toBe(false)

    // Custom adapter: no per-session model picker, like OpenClaw/Hermes.
    expect(getAgentAdapterDescriptor('custom')).toMatchObject({
      id: 'custom',
      name: 'Custom ACP Agent',
      defaultModelId: 'default',
      defaultReasoningEffort: 'medium',
      modelControl: 'best-effort',
    })
    expect(getAgentAdapterDescriptor('custom')?.models).toEqual([])
    expect(isSupportedAgentModel('custom', undefined)).toBe(true)
    expect(isSupportedAgentModel('custom', 'default')).toBe(true)
    expect(isSupportedAgentModel('custom', 'anything')).toBe(false)
    expect(isSupportedReasoningEffort('custom', 'low')).toBe(true)
    expect(isSupportedReasoningEffort('custom', 'medium')).toBe(true)
    expect(isSupportedReasoningEffort('custom', 'high')).toBe(true)
    expect(isSupportedReasoningEffort('custom', 'xhigh')).toBe(false)
  })

  it('recognizes custom as a valid adapter', () => {
    expect(isAgentAdapter('custom')).toBe(true)
    expect(isAgentAdapter('unknown')).toBe(false)
    expect(isAgentAdapter(42)).toBe(false)
    expect(isAgentAdapter(null)).toBe(false)
  })
})
