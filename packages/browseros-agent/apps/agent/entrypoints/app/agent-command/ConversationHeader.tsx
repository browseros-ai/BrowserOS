import { ArrowLeft, Home } from 'lucide-react'
import type { FC, ReactNode } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { formatRelativeTime } from '@/entrypoints/app/agents/agent-display.helpers'
import type { HarnessAgent } from '@/entrypoints/app/agents/agent-harness-types'
import { AgentSummaryChips } from '@/entrypoints/app/agents/agent-row/AgentSummaryChips'
import { formatTokens } from '@/entrypoints/app/agents/agent-row/agent-row.helpers'
import type { AgentAdapterHealth } from '@/entrypoints/app/agents/agent-row/agent-row.types'
import { PinToggle } from '@/entrypoints/app/agents/agent-row/PinToggle'
import type { AgentLiveness } from '@/entrypoints/app/agents/LivenessDot'
import { cn } from '@/lib/utils'

interface ConversationHeaderProps {
  agent: HarnessAgent | null
  fallbackName: string
  fallbackAdapter: 'claude' | 'codex' | 'openclaw' | 'unknown'
  adapterHealth: AgentAdapterHealth | null
  backLabel: string
  backTarget: 'home' | 'page'
  onGoHome: () => void
  onPinToggle: (next: boolean) => void
  /** Optional trailing slot — currently used for the Outputs rail toggle. */
  headerExtra?: ReactNode
}

/**
 * Strip above the chat. Mirrors the `/agents` row card's title row +
 * summary chips so the user gets adapter health, pin state, and status
 * at a glance — but adds the meta line (last used · lifetime tokens ·
 * queued) that's specific to this surface.
 *
 * The mobile `lg:hidden` Back button is preserved so the small-screen
 * collapse keeps a navigable header without a sidebar.
 */
export const ConversationHeader: FC<ConversationHeaderProps> = ({
  agent,
  fallbackName,
  fallbackAdapter,
  adapterHealth,
  backLabel,
  backTarget,
  onGoHome,
  onPinToggle,
  headerExtra,
}) => {
  const BackIcon = backTarget === 'home' ? Home : ArrowLeft
  const adapter = agent?.adapter ?? fallbackAdapter
  const status: AgentLiveness = agent?.status ?? 'unknown'
  const lastUsedAt = agent?.lastUsedAt ?? null
  const pinned = agent?.pinned ?? false
  const queueCount = agent?.queue?.length ?? 0
  const tokens = agent?.tokens ?? null
  const lifetimeTotal = tokens
    ? tokens.cumulative.input + tokens.cumulative.output
    : 0

  const metaParts: string[] = []
  if (lastUsedAt !== null) metaParts.push(formatRelativeTime(lastUsedAt))
  if (lifetimeTotal > 0) metaParts.push(`${formatTokens(lifetimeTotal)} tokens`)
  if (queueCount > 0) {
    metaParts.push(queueCount === 1 ? '1 queued' : `${queueCount} queued`)
  }

  return (
    <div className="flex min-h-[60px] shrink-0 items-center justify-between gap-4 px-5 py-2.5">
      <div className="flex min-w-0 items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={onGoHome}
          className="size-8 shrink-0 rounded-xl lg:hidden"
          title={backLabel}
        >
          <BackIcon className="size-4" />
        </Button>
        <div className="group min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate font-semibold text-[15px] leading-6">
              {agent?.name || fallbackName}
            </span>
            {agent ? (
              <PinToggle pinned={pinned} onToggle={onPinToggle} />
            ) : null}
          </div>
          <div className="mt-0.5 flex items-center gap-2">
            <AgentSummaryChips
              adapter={adapter}
              modelLabel={agent?.modelId ?? null}
              reasoningEffort={agent?.reasoningEffort ?? null}
              adapterHealth={adapterHealth}
            />
          </div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        <div className="flex shrink-0 flex-col items-end gap-1">
          <StatusPill
            status={status}
            hasActiveTurn={Boolean(agent?.activeTurnId)}
          />
          <div className="flex h-4 items-center text-[11px] text-muted-foreground">
            <span className="truncate">
              {metaParts.length > 0 ? metaParts.join(' · ') : '\u00A0'}
            </span>
          </div>
        </div>
        {headerExtra ? (
          <div className="flex shrink-0 items-center">{headerExtra}</div>
        ) : null}
      </div>
    </div>
  )
}

interface StatusPillProps {
  status: AgentLiveness
  hasActiveTurn: boolean
}

/**
 * Working / Asleep / Attention all get distinctive styling; idle keeps
 * the legacy emerald `Ready` pill so the default state is visually
 * calm. Defensive working: `idle + activeTurnId` falls through to the
 * working pill since the server says a turn is in flight.
 */
const StatusPill: FC<StatusPillProps> = ({ status, hasActiveTurn }) => {
  const effective: AgentLiveness =
    status === 'idle' && hasActiveTurn ? 'working' : status

  const base =
    'inline-flex items-center gap-2 rounded-full border px-3 py-0.5 text-[11px] uppercase tracking-[0.18em]'

  if (effective === 'working') {
    return (
      <Badge
        variant="secondary"
        className={cn(
          base,
          'border-amber-200 bg-amber-50 text-amber-900 hover:bg-amber-50',
        )}
      >
        <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
        Working
      </Badge>
    )
  }
  if (effective === 'asleep') {
    return (
      <Badge variant="outline" className={cn(base, 'text-muted-foreground')}>
        <span className="size-1.5 rounded-full bg-muted-foreground/50" />
        Asleep
      </Badge>
    )
  }
  if (effective === 'error') {
    return (
      <Badge
        variant="destructive"
        className={cn(base, 'border-destructive/30')}
      >
        <span className="size-1.5 rounded-full bg-destructive-foreground" />
        Attention
      </Badge>
    )
  }
  if (effective === 'idle') {
    return (
      <Badge
        variant="outline"
        className={cn(
          base,
          'border-emerald-200 bg-emerald-50 text-emerald-900 hover:bg-emerald-50',
        )}
      >
        <span className="size-1.5 rounded-full bg-emerald-500" />
        Ready
      </Badge>
    )
  }
  return (
    <Badge variant="outline" className={cn(base, 'text-muted-foreground')}>
      <span className="size-1.5 rounded-full bg-muted-foreground/30" />
      Setup
    </Badge>
  )
}
