import type { HarnessAgent, HarnessAgentAdapter } from './agent-harness-types'
import type { AgentListItem } from './agents-page-types'

function formatHarnessAdapter(adapter: HarnessAgentAdapter): string {
  if (adapter === 'claude') return 'Claude Code'
  return 'Codex'
}

export function toHarnessListItem(agent: HarnessAgent): AgentListItem {
  return {
    key: `agent-harness:${agent.id}`,
    agentId: agent.id,
    name: agent.name,
    source: 'agent-harness',
    runtimeLabel: formatHarnessAdapter(agent.adapter),
    modelLabel: agent.modelId ?? 'default',
    detail: `${agent.adapter}:main`,
    canChat: true,
    canDelete: true,
  }
}
