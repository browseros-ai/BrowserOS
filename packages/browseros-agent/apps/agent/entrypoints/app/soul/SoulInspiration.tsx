import { ExternalLink } from 'lucide-react'
import type { FC } from 'react'

export const SoulInspiration: FC = () => {
  return (
    <div className="rounded-lg border border-border/50 bg-muted/30 px-4 py-3">
      <p className="text-muted-foreground text-xs">
        Inspired by{' '}
        <a
          href="https://soul.md/"
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1 font-medium text-foreground underline-offset-2 hover:underline"
        >
          OpenClaw's SOUL.md
          <ExternalLink className="h-3 w-3" />
        </a>{' '}
        — the idea that Sup should have an evolving personality shaped by their
        conversations.
      </p>
    </div>
  )
}
