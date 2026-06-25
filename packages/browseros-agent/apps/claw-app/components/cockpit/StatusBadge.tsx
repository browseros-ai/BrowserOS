import { type RunStatus, STATUS_META } from '@/lib/status'
import { cn } from '@/lib/utils'

interface StatusBadgeProps {
  status: RunStatus
  className?: string
}

/**
 * Color-coded status pill with optional pulse-dot. Token-driven via
 * STATUS_META so adding a new RunStatus is one entry, not a sweep
 * through the JSX of every consumer.
 */
export function StatusBadge({ status, className }: StatusBadgeProps) {
  const meta = STATUS_META[status]
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-2 py-0.5 font-semibold text-[11px] tracking-wide',
        meta.bgClass,
        meta.textClass,
        className,
      )}
    >
      {meta.pulse && (
        <span
          aria-hidden
          className={cn(
            'size-1.5 shrink-0 animate-pulse-dot rounded-full bg-current',
          )}
        />
      )}
      {meta.label}
    </span>
  )
}
