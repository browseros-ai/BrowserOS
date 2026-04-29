import type { FC } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

export type AgentLiveness = 'working' | 'idle' | 'asleep' | 'error' | 'unknown'

interface LivenessDotProps {
  status: AgentLiveness
  /**
   * Optional human-friendly secondary line, e.g. "Idle for 4 min" or
   * "Asleep — no activity for 22 min". When absent the tooltip just
   * reads the status label.
   */
  detail?: string
  className?: string
}

const VARIANT: Record<
  AgentLiveness,
  { dot: string; ring: string; label: string }
> = {
  working: {
    // Animated amber pulse + soft halo so the eye catches an active
    // agent in a long list without the dot screaming for attention.
    dot: 'bg-amber-500 animate-pulse',
    ring: 'ring-2 ring-amber-200',
    label: 'Working on a turn',
  },
  idle: {
    dot: 'bg-emerald-500',
    ring: 'ring-2 ring-emerald-100',
    label: 'Idle',
  },
  asleep: {
    dot: 'bg-muted-foreground/40',
    ring: 'ring-2 ring-muted',
    label: 'Asleep',
  },
  error: {
    dot: 'bg-destructive',
    ring: 'ring-2 ring-destructive/30',
    label: 'Attention',
  },
  unknown: {
    dot: 'bg-muted-foreground/30',
    ring: 'ring-2 ring-muted',
    label: 'Status unknown',
  },
}

export const LivenessDot: FC<LivenessDotProps> = ({
  status,
  detail,
  className,
}) => {
  const variant = VARIANT[status]
  return (
    <TooltipProvider delayDuration={150}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span
            role="img"
            aria-label={detail ?? variant.label}
            className={cn(
              'inline-block h-3 w-3 rounded-full',
              variant.dot,
              variant.ring,
              className,
            )}
          />
        </TooltipTrigger>
        <TooltipContent side="right" className="text-xs">
          {detail ?? variant.label}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
