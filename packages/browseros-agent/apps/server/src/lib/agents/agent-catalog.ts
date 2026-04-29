/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentAdapter, AgentAdapterDescriptor } from './agent-types'

export const AGENT_ADAPTER_CATALOG: AgentAdapterDescriptor[] = [
  {
    id: 'claude',
    name: 'Claude Code',
    defaultModelId: 'haiku',
    defaultReasoningEffort: 'medium',
    modelControl: 'best-effort',
    models: [
      { id: 'opus', label: 'Opus' },
      { id: 'sonnet', label: 'Sonnet' },
      { id: 'haiku', label: 'Haiku', recommended: true },
    ],
    reasoningEfforts: [
      { id: 'low', label: 'Low' },
      { id: 'medium', label: 'Medium', recommended: true },
      { id: 'high', label: 'High' },
      { id: 'xhigh', label: 'Extra high' },
      { id: 'max', label: 'Max' },
    ],
  },
  {
    id: 'codex',
    name: 'Codex',
    defaultModelId: 'gpt-5.5',
    defaultReasoningEffort: 'medium',
    modelControl: 'best-effort',
    models: [{ id: 'gpt-5.5', label: 'GPT-5.5', recommended: true }],
    reasoningEfforts: [
      { id: 'low', label: 'Low' },
      { id: 'medium', label: 'Medium', recommended: true },
      { id: 'high', label: 'High' },
      { id: 'xhigh', label: 'Extra high' },
    ],
  },
]

export function getAgentAdapterDescriptor(
  adapter: AgentAdapter,
): AgentAdapterDescriptor | null {
  return AGENT_ADAPTER_CATALOG.find((entry) => entry.id === adapter) ?? null
}

export function isAgentAdapter(value: unknown): value is AgentAdapter {
  return value === 'claude' || value === 'codex'
}

export function resolveDefaultModelId(adapter: AgentAdapter): string {
  return getAgentAdapterDescriptor(adapter)?.defaultModelId ?? 'default'
}

export function resolveDefaultReasoningEffort(adapter: AgentAdapter): string {
  return getAgentAdapterDescriptor(adapter)?.defaultReasoningEffort ?? 'medium'
}

export function isSupportedAgentModel(
  adapter: AgentAdapter,
  modelId: string | undefined,
): boolean {
  if (!modelId || modelId === 'default') return true
  const descriptor = getAgentAdapterDescriptor(adapter)
  return Boolean(descriptor?.models.some((model) => model.id === modelId))
}

export function isSupportedReasoningEffort(
  adapter: AgentAdapter,
  effort: string | undefined,
): boolean {
  if (!effort) return true
  const descriptor = getAgentAdapterDescriptor(adapter)
  return Boolean(
    descriptor?.reasoningEfforts.some((option) => option.id === effort),
  )
}
