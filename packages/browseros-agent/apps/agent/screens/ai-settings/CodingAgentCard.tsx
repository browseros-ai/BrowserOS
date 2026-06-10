import { Check, Loader2, Trash2 } from 'lucide-react'
import type { FC } from 'react'
import type { HarnessAgentAdapter } from '@/modules/agents/agent-harness-types'
import type { AgentListItem } from '@/modules/agents/agents-page-types'
import { AdapterIcon, adapterLabel } from '../../components/agents/AdapterIcon'
import {
  canDelete as canDeleteAgent,
  displayName,
} from '../../components/agents/agent-display.helpers'
import { Badge } from '../../components/ui/badge'
import { Button } from '../../components/ui/button'
import { cn } from '../../lib/utils'

interface CodingAgentCardProps {
  agent: AgentListItem
  adapter: HarnessAgentAdapter | 'unknown'
  modelLabel: string | null
  reasoningEffort: string | null
  isSelected: boolean
  deleting: boolean
  onSelect: () => void
  onDelete: (agent: AgentListItem) => void
}

/**
 * Provider-style row for coding agents in the AI settings pane. Participates
 * in the same `default-provider` radio group as `ProviderCard` so providers
 * and agents form one exclusive default-target choice.
 */
export const CodingAgentCard: FC<CodingAgentCardProps> = ({
  agent,
  adapter,
  modelLabel,
  reasoningEffort,
  isSelected,
  deleting,
  onSelect,
  onDelete,
}) => {
  const name = displayName(agent)
  const metadata = [adapterLabel(adapter), modelLabel, reasoningEffort]
    .filter((part): part is string => Boolean(part))
    .join(' · ')
  const allowDelete = canDeleteAgent(agent)
  const inputId = `agent-${agent.agentId}`

  return (
    <label
      htmlFor={inputId}
      className={cn(
        'group flex w-full cursor-pointer items-center gap-4 rounded-xl border p-4 text-left transition-all',
        isSelected
          ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]/5 shadow-md'
          : 'border-border bg-card hover:border-[var(--accent-orange)]/50 hover:shadow-sm',
      )}
    >
      <input
        type="radio"
        id={inputId}
        name="default-provider"
        className="sr-only"
        checked={isSelected}
        onChange={() => onSelect()}
      />
      <div
        className={cn(
          'flex h-5 w-5 shrink-0 items-center justify-center rounded-full border-2 transition-all',
          isSelected
            ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]'
            : 'border-border',
        )}
      >
        {isSelected && <Check className="h-3 w-3 text-white" />}
      </div>
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
        <AdapterIcon adapter={adapter} className="h-6 w-6" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex items-center gap-2">
          <span className="truncate font-semibold">{name}</span>
          {isSelected && (
            <Badge
              variant="secondary"
              className="rounded bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]"
            >
              DEFAULT
            </Badge>
          )}
        </div>
        <p className="truncate text-muted-foreground text-sm">{metadata}</p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Delete ${name}`}
          disabled={!allowDelete || deleting}
          onClick={() => onDelete(agent)}
          className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
        >
          {deleting ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Trash2 className="h-4 w-4" />
          )}
        </Button>
      </div>
    </label>
  )
}
