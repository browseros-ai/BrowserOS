import { Bot, Plus } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'

interface AgentsEmptyStateProps {
  onCreateAgent: () => void
}

export const AgentsEmptyState: FC<AgentsEmptyStateProps> = ({
  onCreateAgent,
}) => {
  return (
    <div className="rounded-xl border border-border border-dashed bg-card/50 p-12 text-center">
      <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
        <Bot className="h-6 w-6 text-[var(--accent-orange)]" />
      </div>
      <h3 className="mb-1 font-semibold">No agents yet</h3>
      <p className="mx-auto mb-4 max-w-sm text-muted-foreground text-sm">
        Spin up an OpenClaw, Claude Code, or Codex agent to chat with, schedule,
        or run in the background.
      </p>
      <Button
        onClick={onCreateAgent}
        variant="outline"
        className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
      >
        <Plus className="mr-1.5 h-4 w-4" />
        Create your first agent
      </Button>
    </div>
  )
}
