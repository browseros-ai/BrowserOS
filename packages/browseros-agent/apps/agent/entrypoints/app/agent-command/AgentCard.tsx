import { Bot, Loader2, Wrench } from 'lucide-react'
import type { FC } from 'react'
import type { AgentCardData } from '@/lib/agent-conversations/types'
import { cn } from '@/lib/utils'

interface AgentCardProps {
  agent: AgentCardData
  onClick: () => void
  active?: boolean
}

function formatTimestamp(timestamp?: number): string {
  if (!timestamp) return 'No activity yet'
  const diff = Date.now() - timestamp
  const minutes = Math.floor(diff / 60000)
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  return `${Math.floor(hours / 24)}d ago`
}

function getStatusLabel(status: AgentCardData['status']): string {
  if (status === 'working') return 'Working'
  if (status === 'error') return 'Error'
  return 'Ready'
}

function getStatusTone(status: AgentCardData['status']): string {
  if (status === 'working') return 'bg-amber-500'
  if (status === 'error') return 'bg-destructive'
  return 'bg-emerald-500'
}

function formatCost(usd: number): string {
  if (usd < 0.005) return `$${usd.toFixed(4)}`
  return `$${usd.toFixed(2)}`
}

export const AgentCardExpanded: FC<AgentCardProps> = ({
  agent,
  onClick,
  active,
}) => (
  <button
    type="button"
    onClick={onClick}
    className={cn(
      'group flex min-h-32 w-full min-w-0 flex-col rounded-2xl border p-4 text-left shadow-sm transition-all duration-200',
      active
        ? 'border-border/80 bg-card shadow-md ring-1 ring-[var(--accent-orange)]/20'
        : 'border-border/60 bg-card/85 hover:border-border hover:bg-card hover:shadow-md',
    )}
  >
    <div className="flex items-start justify-between gap-3">
      <div className="flex min-w-0 items-center gap-3">
        <div
          className={cn(
            'flex size-10 shrink-0 items-center justify-center rounded-xl',
            active
              ? 'bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]'
              : 'bg-muted text-muted-foreground',
          )}
        >
          <Bot className="size-5" />
        </div>
        <div className="min-w-0">
          <div className="truncate font-semibold text-sm">{agent.name}</div>
          <div className="truncate text-muted-foreground text-xs">
            {agent.model ?? 'OpenClaw agent'}
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2 rounded-full border border-border/60 bg-background/70 px-2.5 py-1 text-[11px] text-muted-foreground">
        <span
          className={cn('size-2 rounded-full', getStatusTone(agent.status))}
        />
        <span>{getStatusLabel(agent.status)}</span>
      </div>
    </div>

    <div className="mt-4 flex-1">
      <p className="line-clamp-2 text-foreground/90 text-sm">
        {agent.lastMessage ??
          'Start a conversation to see recent work and summaries.'}
      </p>
    </div>

    <div className="mt-4 space-y-1.5 text-muted-foreground text-xs">
      <div className="flex items-center justify-between gap-3">
        <span>{formatTimestamp(agent.lastMessageTimestamp)}</span>
        {agent.costUsd ? (
          <span className="tabular-nums opacity-70">
            {formatCost(agent.costUsd)}
          </span>
        ) : null}
      </div>
      {agent.status === 'working' && agent.currentTool ? (
        <div className="flex items-center gap-1.5 text-[var(--accent-orange)]/70">
          <Loader2 className="size-3 shrink-0 animate-spin" />
          <span className="truncate">{agent.currentTool}</span>
        </div>
      ) : agent.activitySummary ? (
        <div className="flex items-center gap-1.5 text-muted-foreground/60">
          <Wrench className="size-3 shrink-0" />
          <span className="truncate">{agent.activitySummary}</span>
        </div>
      ) : null}
    </div>
  </button>
)

export const AgentCardCompact: FC<AgentCardProps> = ({
  agent,
  onClick,
  active,
}) => (
  <button
    type="button"
    onClick={onClick}
    className={cn(
      'inline-flex items-center gap-2 rounded-full border px-3 py-2 text-sm transition-colors',
      active
        ? 'border-border bg-card shadow-sm ring-1 ring-[var(--accent-orange)]/20'
        : 'border-border/60 bg-card/85 text-foreground hover:border-border hover:bg-card',
    )}
  >
    <span
      className={cn(
        'size-2 rounded-full',
        active ? 'bg-[var(--accent-orange)]' : getStatusTone(agent.status),
      )}
    />
    <span className="truncate">{agent.name}</span>
  </button>
)
