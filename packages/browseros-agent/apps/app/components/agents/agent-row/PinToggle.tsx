import { Star } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

export interface PinToggleProps {
  pinned: boolean
  onToggle: (next: boolean) => void
}

/**
 * Trailing star toggle. The button is *always rendered* — only its
 * opacity changes between pinned/unpinned/hover states — so the title
 * row's height is constant. Hiding the slot via `display: none` would
 * collapse the row's vertical metrics on hover and shift every card
 * below in the rail.
 *
 * Placement is trailing the title (after the status badge) so the
 * title itself flushes left regardless of pin state — leading the
 * row with the star would indent the title relative to the model /
 * preview / meta lines beneath it.
 */
export const PinToggle: FC<PinToggleProps> = ({ pinned, onToggle }) => (
  <TooltipProvider delayDuration={300}>
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            'size-6 text-muted-foreground transition-opacity hover:text-foreground',
            pinned ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
          )}
          aria-pressed={pinned}
          aria-label={pinned ? 'Unpin agent' : 'Pin agent'}
          onClick={(event) => {
            event.stopPropagation()
            onToggle(!pinned)
          }}
        >
          <Star
            className={cn(
              'size-3.5',
              pinned && 'fill-amber-400 text-amber-500',
            )}
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top" className="text-xs">
        {pinned ? 'Unpin' : 'Pin to top'}
      </TooltipContent>
    </Tooltip>
  </TooltipProvider>
)
