import type { CockpitStats } from '@browseros/claw-api'
import { ArrowUpRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'
import { formatStatGlance } from './stat-glance.helpers'

interface StatGlanceProps {
  stats?: CockpitStats
  isPending?: boolean
  /** The live header collapses the line to just tokens saved so it never buries. */
  compact?: boolean
}

/** The one compact value line the reader wants at a glance, in both dock states. */
export function StatGlance({ stats, isPending, compact }: StatGlanceProps) {
  if (isPending) return <Skeleton className="h-4 w-44" />
  if (!stats?.hasMeasuredStats) return null

  const parts = formatStatGlance(stats.allTime)

  if (compact) {
    return (
      <span className="font-mono text-ink-2 text-sm tabular-nums">
        {parts.tokensSaved} saved
      </span>
    )
  }

  return (
    <span className="inline-flex flex-wrap items-center gap-x-2 gap-y-0.5 text-ink-2 text-sm">
      <span>
        <span className="font-mono text-ink tabular-nums">
          {parts.tokensSaved}
        </span>{' '}
        tokens saved,{' '}
        <span className="font-mono text-ink tabular-nums">
          {parts.timeSaved}
        </span>{' '}
        saved,{' '}
        <span className="font-mono text-ink tabular-nums">
          {parts.percentFewer}%
        </span>{' '}
        fewer
      </span>
      <NavLink
        className="group inline-flex items-center gap-0.5 text-accent-ink hover:underline"
        to="/audit"
      >
        stats
        <ArrowUpRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
      </NavLink>
    </span>
  )
}
