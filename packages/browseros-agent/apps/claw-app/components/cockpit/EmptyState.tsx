import { Activity } from 'lucide-react'
import type { ReactNode } from 'react'

interface EmptyStateProps {
  title: string
  hint: ReactNode
  icon?: ReactNode
}

/**
 * Shared empty-state card used by RunningGrid and RecentActivity when
 * the registry has nothing to show. Same border / padding rhythm as
 * the running card so the page does not collapse to a flat header
 * when no agents are connected.
 */
export function EmptyState({ title, hint, icon }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-2xl border border-border-2 bg-card p-8 text-center">
      <span
        aria-hidden
        className="flex size-10 items-center justify-center rounded-xl bg-bg-sunken text-ink-3"
      >
        {icon ?? <Activity className="size-5" />}
      </span>
      <div className="font-bold text-base">{title}</div>
      <div className="max-w-md text-ink-3 text-sm leading-relaxed">{hint}</div>
    </div>
  )
}
