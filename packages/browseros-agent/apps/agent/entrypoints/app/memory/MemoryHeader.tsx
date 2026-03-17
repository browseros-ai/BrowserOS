import { Brain, HelpCircle } from 'lucide-react'
import type { FC } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { memoryHelpUrl } from '@/lib/constants/productUrls'

export const MemoryHeader: FC = () => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-violet-500/10">
          <Brain className="h-6 w-6 text-violet-500" />
        </div>
        <div className="flex-1">
          <div className="mb-1 flex items-center gap-2">
            <h2 className="font-semibold text-xl">Agent Memory</h2>
            <TooltipProvider delayDuration={0}>
              <Tooltip>
                <TooltipTrigger asChild>
                  <a
                    href={memoryHelpUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="rounded-full p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                  >
                    <HelpCircle className="h-4 w-4" />
                  </a>
                </TooltipTrigger>
                <TooltipContent>Learn more about memory</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <p className="text-muted-foreground text-sm">
            Facts your agent remembers about you — your name, preferences,
            projects, and tools. Edit directly or teach through conversation.
          </p>
        </div>
      </div>
    </div>
  )
}
