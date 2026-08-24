import { ArrowUpRight } from 'lucide-react'
import type { FC } from 'react'
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
  /** Fill the parent form's command field with the chosen starting point. */
  onSelect: (command: string) => void
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
        {POPULAR_ACP_AGENTS.map((agent) => (
          <div
            key={agent.id}
            className="flex items-center gap-3 rounded-lg border border-border bg-background p-3"
          >
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10 font-semibold text-[var(--accent-orange)] text-sm">
              {agent.mark}
            </div>
            <div className="min-w-0 flex-1">
              <p className="font-medium text-sm">{agent.label}</p>
              <p className="text-muted-foreground text-xs">{agent.blurb}</p>
              {agent.suggestedCommand ? (
                <p className="mt-1 break-all font-mono text-muted-foreground text-xs">
                  {agent.suggestedCommand}
                </p>
              ) : null}
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {agent.suggestedCommand ? (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => onSelect(agent.suggestedCommand as string)}
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
        ))}
      </div>

      <p className="text-muted-foreground text-xs">
        Commands are starting points; confirm the exact command and login in
        each agent&rsquo;s ACP docs.
      </p>
    </DialogContent>
  </Dialog>
)
