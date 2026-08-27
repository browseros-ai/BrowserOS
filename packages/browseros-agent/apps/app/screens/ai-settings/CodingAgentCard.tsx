import { Check, Loader2, Pencil, Trash2 } from 'lucide-react'
import type { FC } from 'react'
import { AdapterIcon, adapterLabel } from '@/components/agents/AdapterIcon'
import {
  agentBrandKey,
  BRAND_MARKS,
} from '@/components/agents/agent-brand-marks'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'

export interface CodingAgentCardProps {
  agent: AcpAgent
  isSelected: boolean
  deleting: boolean
  onSelect: () => void
  onDelete: (agent: AcpAgent) => void
  onEdit?: (agent: AcpAgent) => void
}

export const CodingAgentCard: FC<CodingAgentCardProps> = ({
  agent,
  isSelected,
  deleting,
  onSelect,
  onDelete,
  onEdit,
}) => {
  const isCustom = agent.type === 'custom'
  const Mark = BRAND_MARKS[agentBrandKey(agent) ?? '']
  const primaryLabel =
    isCustom && agent.customConfig?.command
      ? agent.customConfig.command
      : adapterLabel(agent.type)
  const metadata = [
    primaryLabel,
    agent.modelId ?? 'Agent default model',
    agent.reasoningEffort,
  ]
    .filter((part): part is string => Boolean(part))
    .join(' · ')
  const inputId = `agent-${agent.id}`

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
        onChange={onSelect}
      />
      <div
        className={cn(
          'flex h-5 w-5 shrink-0 items-center justify-center rounded-full border-2 transition-all',
          isSelected
            ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]'
            : 'border-border',
        )}
      >
        {isSelected ? <Check className="h-3 w-3 text-white" /> : null}
      </div>
      <div className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
        {Mark ? (
          <Mark className="h-6 w-6" />
        ) : (
          <AdapterIcon adapter={agent.type} className="h-6 w-6" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex items-center gap-2">
          <span className="truncate font-semibold">{agent.name}</span>
          {isSelected ? (
            <Badge
              variant="secondary"
              className="rounded bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]"
            >
              DEFAULT
            </Badge>
          ) : null}
        </div>
        <p className="truncate text-muted-foreground text-sm">{metadata}</p>
      </div>
      {isCustom && onEdit ? (
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Edit ${agent.name}`}
          onClick={(event) => {
            event.preventDefault()
            onEdit(agent)
          }}
          className="shrink-0 text-muted-foreground hover:bg-[var(--accent-orange)]/10 hover:text-[var(--accent-orange)]"
        >
          <Pencil className="h-4 w-4" />
        </Button>
      ) : null}
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Delete ${agent.name}`}
        disabled={deleting}
        onClick={(event) => {
          event.preventDefault()
          onDelete(agent)
        }}
        className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
      >
        {deleting ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Trash2 className="h-4 w-4" />
        )}
      </Button>
    </label>
  )
}
