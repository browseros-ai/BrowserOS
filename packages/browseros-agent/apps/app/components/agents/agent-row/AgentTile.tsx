import type { FC } from 'react'
import { cn } from '@/lib/utils'
import type { HarnessAgentAdapter } from '@/modules/agents/agent-harness-types'
import { AdapterIcon } from '../AdapterIcon'
import { livenessDetail } from '../agent-display.helpers'
import { type AgentLiveness, LivenessDot } from '../LivenessDot'

export interface AgentTileProps {
  adapter: HarnessAgentAdapter | 'unknown'
  status: AgentLiveness
  lastUsedAt: number | null
}

/**
 * Adapter glyph + a single liveness dot. Adapter health is no longer
 * surfaced here — it lives as an inline pill inside `AgentSummaryChips`
 * so the user isn't asked to disambiguate two dots on the same tile.
 */
export const AgentTile: FC<AgentTileProps> = ({
  adapter,
  status,
  lastUsedAt,
}) => (
  <div className="relative shrink-0">
    <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-muted text-muted-foreground">
      <AdapterIcon adapter={adapter} className="h-6 w-6" />
    </div>
    <LivenessDot
      status={status}
      detail={livenessDetail(status, lastUsedAt)}
      className={cn(
        'absolute -right-0.5 -bottom-0.5',
        status === 'working' && 'animate-pulse',
      )}
    />
  </div>
)
