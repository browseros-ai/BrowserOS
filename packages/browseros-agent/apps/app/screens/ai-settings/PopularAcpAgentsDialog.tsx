import { ArrowUpRight } from 'lucide-react'
import type { FC } from 'react'
import { BRAND_MARKS } from '@/components/agents/agent-brand-marks'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { POPULAR_ACP_AGENTS } from './popular-acp-agents'

export interface PopularAcpAgentsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /**
   * Fill the parent form's command field with the chosen starting point and
   * carry the agent id so the saved agent adopts that agent's brand logo.
   */
  onSelect: (command: string, agentId: string) => void
}

export const PopularAcpAgentsDialog: FC<PopularAcpAgentsDialogProps> = ({
  open,
  onOpenChange,
  onSelect,
}) => (
  <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent className="max-h-[80vh] overflow-y-auto">
      <DialogHeader>
        <DialogTitle>Popular ACP agents</DialogTitle>
        <DialogDescription>
          Pick a starting point. Check each agent&rsquo;s docs for install and
          login.
        </DialogDescription>
      </DialogHeader>

      <div className="space-y-2 py-2">
        {POPULAR_ACP_AGENTS.map((agent) => {
          const Mark = BRAND_MARKS[agent.id]
          return (
            <div
              key={agent.id}
              className="flex items-center gap-3 rounded-lg border border-border bg-background p-3"
            >
              <div className="flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-card text-foreground ring-1 ring-border">
                {Mark ? (
                  <Mark className="h-6 w-6" />
                ) : (
                  <span className="font-semibold text-muted-foreground text-sm">
                    {agent.mark}
                  </span>
                )}
              </div>
              <div className="min-w-0 flex-1">
                <p className="font-medium text-sm">{agent.label}</p>
                <p className="text-muted-foreground text-xs">{agent.blurb}</p>
                {agent.suggestedCommand ? (
                  <code className="mt-1.5 inline-flex w-fit max-w-full break-all rounded-md border border-border bg-muted px-2 py-1 font-mono text-[11px] text-foreground/80">
                    {agent.suggestedCommand}
                  </code>
                ) : null}
              </div>
              <div className="flex shrink-0 items-center gap-2">
                {agent.suggestedCommand ? (
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      onSelect(agent.suggestedCommand as string, agent.id)
                    }
                  >
                    Use
                  </Button>
                ) : null}
                <a
                  href={agent.docsUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-0.5 font-medium text-[var(--accent-orange)] text-xs"
                >
                  Docs <ArrowUpRight className="h-3 w-3" />
                </a>
              </div>
            </div>
          )
        })}
      </div>

      <p className="text-muted-foreground text-xs">
        Commands are starting points; confirm the exact command and login in
        each agent&rsquo;s ACP docs.
      </p>
    </DialogContent>
  </Dialog>
)
