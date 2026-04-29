import { Bot, Plus } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'

interface AgentsHeaderProps {
  onCreateAgent: () => void
}

/**
 * Mirrors the visual shape of `SoulHeader` and `ScheduledTasksHeader`
 * so the page reads as part of the same family. Loose lifecycle
 * controls that used to sit next to the title moved into
 * `GatewayStatusBar` — they're OpenClaw-specific and don't apply to
 * Claude/Codex agents.
 */
export const AgentsHeader: FC<AgentsHeaderProps> = ({ onCreateAgent }) => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Bot className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <h2 className="mb-1 font-semibold text-xl">Agents</h2>
          <p className="text-muted-foreground text-sm">
            OpenClaw, Claude Code, and Codex agents — chat, schedule, and run
            them in the background.
          </p>
        </div>
        <Button
          onClick={onCreateAgent}
          className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
          variant="outline"
        >
          <Plus className="mr-1.5 h-4 w-4" />
          New Agent
        </Button>
      </div>
    </div>
  )
}
