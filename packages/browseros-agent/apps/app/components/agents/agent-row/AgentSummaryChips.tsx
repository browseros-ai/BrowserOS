import { TriangleAlert } from 'lucide-react'
import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from '@/components/ui/hover-card'
import { cn } from '@/lib/utils'
import {
  adapterHealthLabel,
  adapterHealthTone,
} from '@/modules/agents/adapter-health'
import type { HarnessAgentAdapter } from '@/modules/agents/agent-harness-types'
import { adapterLabel } from '../AdapterIcon'
import type { AgentAdapterHealth } from './agent-row.types'

export interface AgentSummaryChipsProps {
  adapter: HarnessAgentAdapter | 'unknown'
  modelLabel: string | null
  reasoningEffort: string | null
  /** When unhealthy, the adapter label dims and a warning chip appears. */
  adapterHealth: AgentAdapterHealth | null
}

/**
 * Adapter / model / reasoning summary line. Surfaces adapter health
 * only when unhealthy, keeping the default state visually quiet.
 */
export const AgentSummaryChips: FC<AgentSummaryChipsProps> = ({
  adapter,
  modelLabel,
  reasoningEffort,
  adapterHealth,
}) => {
  const parts = [adapterLabel(adapter)]
  if (modelLabel) parts.push(modelLabel)
  if (reasoningEffort) parts.push(reasoningEffort)
  const unhealthy = adapterHealth?.healthy === false
  const tone = adapterHealth ? adapterHealthTone(adapterHealth) : 'ready'
  return (
    <div
      className={cn(
        'flex items-center gap-1.5 text-muted-foreground text-xs',
        unhealthy && 'text-muted-foreground/70',
      )}
    >
      <span className="truncate">{parts.join(' · ')}</span>
      {unhealthy && adapterHealth && (
        <HoverCard openDelay={200}>
          <HoverCardTrigger asChild>
            <Badge
              variant="outline"
              className="h-5 cursor-default gap-1 border-amber-500/40 bg-amber-50 px-1.5 text-amber-900 hover:bg-amber-50"
            >
              <TriangleAlert className="size-2.5" />
              <span className="font-normal">
                {adapterHealthLabel(adapterHealth)}
              </span>
            </Badge>
          </HoverCardTrigger>
          <HoverCardContent side="right" className="w-72 text-sm">
            <div className="font-medium">
              {adapterLabel(adapter)}{' '}
              {tone === 'danger' ? 'needs setup' : 'warning'}
            </div>
            <div className="mt-1 text-muted-foreground text-xs">
              {adapterHealth.reason ??
                'Adapter binary missing on $PATH. Install it from the adapter docs to use this agent.'}
            </div>
          </HoverCardContent>
        </HoverCard>
      )}
    </div>
  )
}
