import { Plus } from 'lucide-react'
import { Link } from 'react-router'

/**
 * TODO(v2-restore-multi-agent): v2 has no per-agent profile directory,
 * so "New profile" links to a route the v2 router no longer registers
 * (the `/agents/new` route only mounts when VITE_COCKPIT_LEGACY_UI=1).
 * The running grid no longer renders this tile in the default v2
 * build. The component returns when the per-agent story does, which
 * is post the SQLite audit work.
 *
 * Add-tile that sits as the last item in the running grid. Reads as
 * adding to the set rather than a floating CTA.
 */
export function AddAgentTile() {
  return (
    <Link
      to="/agents/new"
      title="New profile - connect a harness and choose what it can touch"
      className="group flex min-h-[260px] cursor-pointer flex-col items-center justify-center gap-2 rounded-2xl border-2 border-border-strong border-dashed bg-transparent p-4 text-center text-ink-3 transition hover:border-accent hover:bg-accent-tint hover:text-accent"
    >
      <span className="flex size-8 items-center justify-center rounded-lg border-2 border-current">
        <Plus className="size-4" />
      </span>
      <div className="font-bold text-[13.5px] text-ink-2 group-hover:text-accent">
        New profile
      </div>
      <div className="text-[11.5px] text-ink-4 leading-tight">
        harness . logins . guardrails
      </div>
    </Link>
  )
}
