import { HelpCircle, Sparkles } from 'lucide-react'
import type { FC } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { soulHelpUrl } from '@/lib/constants/productUrls'

export const SoulHeader: FC = () => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Sparkles className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <div className="mb-1 flex items-center gap-2">
            <h2 className="font-semibold text-xl">Agent Soul</h2>
            <TooltipProvider delayDuration={0}>
              <Tooltip>
                <TooltipTrigger asChild>
                  <a
                    href={soulHelpUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="rounded-full p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                  >
                    <HelpCircle className="h-4 w-4" />
                  </a>
                </TooltipTrigger>
                <TooltipContent>Learn more about SOUL.md</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <p className="text-muted-foreground text-sm">
            Your agent's personality, tone, and behavioral rules. The soul
            evolves as your agent learns how you like to interact.
          </p>
        </div>
      </div>
    </div>
  )
}
