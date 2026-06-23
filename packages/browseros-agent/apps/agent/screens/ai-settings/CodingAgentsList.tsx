import { type FC, useMemo } from 'react'
import { InlineErrorAlert } from '@/components/agents/PageAlerts'
import type { HarnessAgentAdapter } from '@/modules/agents/agent-harness-types'
import { compareAgentsByPinThenRecency } from '@/modules/agents/agents-list-order'
import type { AgentListItem } from '@/modules/agents/agents-page-types'
import { CodingAgentCard } from './CodingAgentCard'
import type { CodingAgentsController } from './coding-agents.hooks'

export interface CodingAgentsListProps {
  controller: CodingAgentsController
  selectedAgentId: string | null
  onSelectAgent: (agentId: string) => void
}

/** Provider-list placement for Claude Code / Codex agents in AI settings. */
export const CodingAgentsList: FC<CodingAgentsListProps> = ({
  controller,
  selectedAgentId,
  onSelectAgent,
}) => {
  const {
    listItems,
    activity,
    harnessAgentLookup,
    pageError,
    dismissPageError,
    deletingAgentKey,
    deleteIsPending,
    handleDelete,
  } = controller

  const ordered = useMemo(() => {
    const withMeta = listItems.map((agent) => {
      const harness = harnessAgentLookup.get(agent.agentId)
      return {
        agent,
        id: agent.agentId,
        pinned: harness?.pinned ?? false,
        lastUsedAt:
          activity[agent.agentId]?.lastUsedAt ?? harness?.lastUsedAt ?? null,
      }
    })
    return withMeta
      .sort(compareAgentsByPinThenRecency)
      .map((entry) => entry.agent)
  }, [activity, harnessAgentLookup, listItems])

  if (ordered.length === 0 && !pageError) return null

  return (
    <div className="space-y-3">
      {pageError ? (
        <InlineErrorAlert message={pageError} onDismiss={dismissPageError} />
      ) : null}
      {ordered.map((agent) => {
        const harness = harnessAgentLookup.get(agent.agentId)
        const adapter = harness?.adapter ?? inferAdapterFromLabel(agent)
        return (
          <CodingAgentCard
            key={agent.key}
            agent={agent}
            adapter={adapter}
            modelLabel={deriveModelLabel(agent, harness?.modelId)}
            reasoningEffort={harness?.reasoningEffort ?? null}
            isSelected={selectedAgentId === agent.agentId}
            deleting={deleteIsPending && deletingAgentKey === agent.key}
            onSelect={() => onSelectAgent(agent.agentId)}
            onDelete={(item) => {
              void handleDelete(item)
            }}
          />
        )
      })}
    </div>
  )
}

function inferAdapterFromLabel(
  agent: AgentListItem,
): HarnessAgentAdapter | 'unknown' {
  const lower = agent.runtimeLabel?.toLowerCase()
  if (lower === 'claude code') return 'claude'
  if (lower === 'codex') return 'codex'
  return 'unknown'
}

function deriveModelLabel(
  agent: AgentListItem,
  harnessModelId: string | undefined,
): string | null {
  if (agent.modelLabel && agent.modelLabel !== 'default') {
    return agent.modelLabel
  }
  return harnessModelId ?? null
}
