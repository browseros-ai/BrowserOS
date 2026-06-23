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
} from '../../../src/lib/agents/adapters/catalog'

describe('AGENT_ADAPTER_CATALOG', () => {
  it('exposes shipped adapters with model and effort options', () => {
    expect(AGENT_ADAPTER_CATALOG.map((adapter) => adapter.id)).toEqual([
      'claude',
      'codex',
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

    expect(isSupportedAgentModel('claude', 'haiku')).toBe(true)
    expect(isSupportedAgentModel('claude', 'claude-opus-4-7')).toBe(true)
    expect(isSupportedAgentModel('claude', 'claude-sonnet-4-6')).toBe(true)
    expect(isSupportedAgentModel('claude', 'claude-haiku-4-5')).toBe(true)
    expect(isSupportedAgentModel('claude', 'claude-not-real')).toBe(false)
    expect(isSupportedAgentModel('codex', 'gpt-5.5')).toBe(true)
    expect(isSupportedAgentModel('codex', 'gpt-5.4-mini')).toBe(true)
    expect(isSupportedAgentModel('codex', 'codex-auto-review')).toBe(false)

    expect(isSupportedReasoningEffort('codex', 'xhigh')).toBe(true)
    expect(isSupportedReasoningEffort('claude', 'banana')).toBe(false)
  })
})
