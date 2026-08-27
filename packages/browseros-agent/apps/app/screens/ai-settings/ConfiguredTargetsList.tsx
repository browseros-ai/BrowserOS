import type { FC } from 'react'
import { AdapterIcon } from '@/components/agents/AdapterIcon'
import {
  agentBrandKey,
  BRAND_MARKS,
} from '@/components/agents/agent-brand-marks'
import { InlineErrorAlert } from '@/components/agents/PageAlerts'
import { BrowserOSIcon, ProviderIcon } from '@/lib/llm-providers/providerIcons'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { ConfiguredTargetRow } from './ConfiguredTargetRow'
import type { CodingAgentsController } from './coding-agents.hooks'
import {
  agentDescription,
  buildAgentActions,
  buildProviderActions,
  providerDescription,
} from './configured-targets.helpers'

export interface ConfiguredTargetsListProps {
  providers: LlmProviderConfig[]
  coding: CodingAgentsController
  selectedProviderId: string | null
  selectedAgentId: string | null
  testingProviderId: string | null
  onSelectProvider: (providerId: string) => void
  onSelectAgent: (agentId: string) => void
  onTestProvider: (provider: LlmProviderConfig) => void
  onEditProvider: (provider: LlmProviderConfig) => void
  onDeleteProvider: (provider: LlmProviderConfig) => void
  onEditAgent?: (agent: AcpAgent) => void
}

/**
 * The providers and the coding agents in one list. They were two components
 * rendering the same row shape into the same column, and they already shared
 * one radio group name, so the split only ever showed up as a visual seam.
 */
export const ConfiguredTargetsList: FC<ConfiguredTargetsListProps> = ({
  providers,
  coding,
  selectedProviderId,
  selectedAgentId,
  testingProviderId,
  onSelectProvider,
  onSelectAgent,
  onTestProvider,
  onEditProvider,
  onDeleteProvider,
  onEditAgent,
}) => {
  const { agents, pageError, dismissPageError, deletingAgentId, handleDelete } =
    coding

  return (
    <div className="space-y-3">
      {pageError ? (
        <InlineErrorAlert message={pageError} onDismiss={dismissPageError} />
      ) : null}

      <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
        {providers.map((provider) => {
          const isBuiltIn = provider.id === 'browseros'
          const isTesting = testingProviderId === provider.id
          const actions = buildProviderActions({
            provider,
            isBuiltIn,
            isTesting,
            onSelectProvider,
            onTestProvider,
            onEditProvider,
            onDeleteProvider,
          })

          return (
            <ConfiguredTargetRow
              key={provider.id}
              id={provider.id}
              name={provider.name}
              description={providerDescription(provider, isBuiltIn)}
              icon={
                isBuiltIn ? (
                  <BrowserOSIcon size={20} />
                ) : (
                  <ProviderIcon type={provider.type} size={20} />
                )
              }
              kind={isBuiltIn ? 'hosted' : 'model'}
              isSelected={selectedProviderId === provider.id}
              busy={isTesting}
              actions={actions}
            />
          )
        })}

        {agents.map((agent) => {
          const Mark = BRAND_MARKS[agentBrandKey(agent) ?? '']
          const actions = buildAgentActions({
            agent,
            isDeleting: deletingAgentId === agent.id,
            onSelectAgent,
            onDeleteAgent: handleDelete,
            onEditAgent,
          })

          return (
            <ConfiguredTargetRow
              key={agent.id}
              id={agent.id}
              name={agent.name}
              description={agentDescription(agent)}
              icon={
                Mark ? (
                  <Mark className="size-5" />
                ) : (
                  <AdapterIcon adapter={agent.type} className="size-5" />
                )
              }
              kind="agent"
              isSelected={selectedAgentId === agent.id}
              busy={deletingAgentId === agent.id}
              actions={actions}
            />
          )
        })}
      </div>
    </div>
  )
}
