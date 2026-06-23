import { Plus } from 'lucide-react'
import { Link } from 'react-router'

/**
 * Add-tile that sits as the last item in the running grid. Reads as
 * adding to the set rather than a floating CTA. Links to
 * /agents/new which currently renders the placeholder; gets replaced
 * with the real wizard in a follow-up.
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
