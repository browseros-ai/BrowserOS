import { Lock } from 'lucide-react'
import type { ActivityRow as ActivityRowData } from '@/modules/api/activity.hooks'
import { ActivityRow } from './ActivityRow'

interface RecentActivityProps {
  rows: ActivityRowData[]
}

/**
 * Cross-agent recent activity log. Lives under the running grid so
 * the user can scan WHAT happened and WHICH agent did it, with the
 * flagged statuses (blocked, needs-human) called out by a chip in
 * the header rather than buried in a long list.
 */
export function RecentActivity({ rows }: RecentActivityProps) {
  const flaggedCount = rows.filter(
    (r) => r.status === 'blocked' || r.status === 'needs-human',
  ).length

  return (
    <section className="space-y-3">
      <div className="flex items-center gap-2.5">
        <h2 className="font-bold text-base">Recent activity</h2>
        {flaggedCount > 0 && (
          <span className="inline-flex items-center gap-1.5 rounded-full bg-red-tint px-2 py-0.5 font-bold text-[11px] text-red">
            <Lock className="size-3" />
            {flaggedCount} flagged
          </span>
        )}
        <div className="flex-1" />
        <span className="text-ink-3 text-xs">Across all agents</span>
      </div>
      <div className="space-y-2">
        {rows.map((row) => (
          <ActivityRow key={row.id} row={row} />
        ))}
      </div>
    </section>
  )
}
