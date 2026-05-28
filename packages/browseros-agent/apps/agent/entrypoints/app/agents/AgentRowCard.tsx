import type { FC } from 'react'
import { cn } from '@/lib/utils'
import { AgentActions } from './agent-row/AgentActions'
import { AgentErrorPanel } from './agent-row/AgentErrorPanel'
import { AgentLastMessage } from './agent-row/AgentLastMessage'
import { AgentMetaRow } from './agent-row/AgentMetaRow'
import { AgentSummaryChips } from './agent-row/AgentSummaryChips'
import { AgentTile } from './agent-row/AgentTile'
import { AgentTitleRow } from './agent-row/AgentTitleRow'
import type {
  AgentRowCallbacks,
  AgentRowData,
} from './agent-row/agent-row.types'

interface AgentRowCardProps extends AgentRowCallbacks {
  data: AgentRowData
  /** Whether THIS agent is mid-delete; renders a spinner in the menu. */
  deleting?: boolean
}

/**
 * Composition shell for the agent rail. Owns no state; sub-components
 * each handle their own micro-state (error-panel collapse, etc.) and
 * emit callbacks (delete, pin/unpin) for the page to act on.
 *
 * The whole card carries state — not just the tile — so the row's
 * border subtly tells the user what's going on at a glance:
 *   working → accent-orange border with a soft glow
 *   error   → destructive border
 *   idle    → muted border, lifts on hover
 */
export const AgentRowCard: FC<AgentRowCardProps> = ({
  data,
  deleting,
  onDelete,
  onPinToggle,
}) => {
  return (
    <div
      className={cn(
        // Layout-stable hover. No translate, no shadow change — both
        // visibly perturb neighbouring rows. Only the border tint
        // shifts on hover, and the rail's vertical rhythm stays
        // exactly the same in every state.
        'group w-full min-w-0 rounded-xl border bg-card p-6 shadow-sm transition-colors',
        data.status === 'working'
          ? 'border-[var(--accent-orange)]/40'
          : data.status === 'error'
            ? 'border-destructive/40'
            : 'border-border hover:border-[var(--accent-orange)]/30',
      )}
    >
      <div className="flex min-w-0 items-start gap-4">
        <AgentTile
          adapter={data.adapter}
          status={data.status}
          lastUsedAt={data.lastUsedAt}
        />

        <div className="min-w-0 flex-1">
          <AgentTitleRow
            agent={data.agent}
            status={data.status}
            pinned={data.pinned}
            turnsByDay={data.turnsByDay}
            failedByDay={data.failedByDay}
            onPinToggle={(next) => onPinToggle(data.agent, next)}
          />

          <AgentSummaryChips
            adapter={data.adapter}
            modelLabel={data.modelLabel}
            reasoningEffort={data.reasoningEffort}
            adapterHealth={data.adapterHealth}
          />

          <AgentLastMessage message={data.lastUserMessage} />

          <AgentMetaRow lastUsedAt={data.lastUsedAt} tokens={data.tokens} />

          {data.status === 'error' && data.lastError && (
            <AgentErrorPanel
              agentId={data.agent.agentId}
              message={data.lastError}
              errorAt={data.lastErrorAt}
            />
          )}
        </div>

        <AgentActions
          agent={data.agent}
          activeTurnId={data.activeTurnId}
          deleting={deleting}
          onDelete={onDelete}
        />
      </div>
    </div>
  )
}
