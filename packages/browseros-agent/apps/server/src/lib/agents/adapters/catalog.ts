/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentAdapter, AgentAdapterDescriptor } from '../agent-types'

export const AGENT_ADAPTER_CATALOG: AgentAdapterDescriptor[] = [
  {
    id: 'claude',
    name: 'Claude Code',
    defaultModelId: 'haiku',
    defaultReasoningEffort: 'medium',
    modelControl: 'best-effort',
    models: [
      { id: 'opus', label: 'Opus (latest)' },
      { id: 'sonnet', label: 'Sonnet (latest)' },
      { id: 'haiku', label: 'Haiku (latest)', recommended: true },
      { id: 'claude-opus-4-7', label: 'Opus 4.7' },
      { id: 'claude-opus-4-6', label: 'Opus 4.6' },
      { id: 'claude-opus-4-5', label: 'Opus 4.5' },
      { id: 'claude-opus-4-1', label: 'Opus 4.1' },
      { id: 'claude-opus-4', label: 'Opus 4' },
      { id: 'claude-sonnet-4-6', label: 'Sonnet 4.6' },
      { id: 'claude-sonnet-4-5', label: 'Sonnet 4.5' },
      { id: 'claude-sonnet-4', label: 'Sonnet 4' },
      { id: 'claude-3-7-sonnet', label: 'Sonnet 3.7' },
      { id: 'claude-3-5-sonnet', label: 'Sonnet 3.5' },
      { id: 'claude-haiku-4-5', label: 'Haiku 4.5' },
      { id: 'claude-3-5-haiku', label: 'Haiku 3.5' },
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
    models: [
      { id: 'gpt-5.5', label: 'GPT-5.5', recommended: true },
      { id: 'gpt-5.4', label: 'GPT-5.4' },
      { id: 'gpt-5.4-mini', label: 'GPT-5.4-Mini' },
      { id: 'gpt-5.3-codex', label: 'GPT-5.3-Codex' },
      { id: 'gpt-5.3-codex-spark', label: 'GPT-5.3-Codex-Spark' },
      { id: 'gpt-5.2', label: 'GPT-5.2' },
    ],
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
