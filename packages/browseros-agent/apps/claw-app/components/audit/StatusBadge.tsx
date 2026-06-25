import { cn } from '@/lib/utils'
import type { TaskStatus } from '@/modules/api/audit.hooks'

interface StatusBadgeProps {
  status: TaskStatus
  className?: string
}

const STYLES: Record<TaskStatus, string> = {
  live: 'bg-accent-tint text-accent',
  done: 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300',
  failed: 'bg-red-500/15 text-red-700 dark:text-red-300',
}

const LABELS: Record<TaskStatus, string> = {
  live: 'Live',
  done: 'Done',
  failed: 'Failed',
}

export function StatusBadge({ status, className }: StatusBadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex items-center rounded-md px-2 py-0.5 font-semibold text-[11px] uppercase tracking-wide',
        STYLES[status],
        status === 'live' && 'animate-pulse',
        className,
      )}
    >
      {status === 'live' && (
        <span className="mr-1 inline-block size-1.5 rounded-full bg-current" />
      )}
      {LABELS[status]}
    </span>
  )
}
