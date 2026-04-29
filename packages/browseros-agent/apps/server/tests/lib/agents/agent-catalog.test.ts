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
  it('exposes Claude and Codex adapters with model and effort options', () => {
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
    expect(isSupportedAgentModel('codex', 'gpt-5.5')).toBe(true)
    expect(isSupportedReasoningEffort('codex', 'xhigh')).toBe(true)
    expect(isSupportedReasoningEffort('claude', 'banana')).toBe(false)
  })
})
