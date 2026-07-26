import type { FC } from 'react'
import { cn } from '@/lib/utils'

export interface PillIndicatorProps {
  text: string
  className?: string
}

/**
 * @public
 */
export const PillIndicator: FC<PillIndicatorProps> = ({
  text,
  className = '',
}) => {
  return (
    <div
      className={cn(
        'mb-4 inline-flex items-center gap-2 rounded-full border border-border/50 bg-muted px-3 py-1.5 text-muted-foreground text-sm',
        className,
      )}
    >
      <span className="relative flex h-2 w-2">
        <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-[var(--accent-orange)] opacity-75"></span>
        <span className="relative inline-flex h-2 w-2 rounded-full bg-[var(--accent-orange)]"></span>
      </span>
      {text}
    </div>
  )
}
