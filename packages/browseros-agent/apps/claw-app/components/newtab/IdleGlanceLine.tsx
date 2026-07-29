import type { CockpitStats } from '@browseros/claw-api'
import { ArrowRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { StatGlance } from './StatGlance'

interface IdleGlanceLineProps {
  stats: CockpitStats | undefined
  pending: boolean
}

/** The single quiet status line below top sites when nothing is running. */
export function IdleGlanceLine({ stats, pending }: IdleGlanceLineProps) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-border-2 border-t pt-3">
      <span className="text-ink-3 text-sm">Idle</span>
      <StatGlance stats={stats} pending={pending} />
      <NavLink
        to="/audit"
        className="group ml-auto inline-flex items-center gap-1 text-ink-3 text-sm transition-colors hover:text-ink"
      >
        activity
        <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
      </NavLink>
    </div>
  )
}
