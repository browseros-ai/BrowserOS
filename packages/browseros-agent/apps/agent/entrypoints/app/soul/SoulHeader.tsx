import { Sparkles } from 'lucide-react'
import type { FC } from 'react'

export const SoulHeader: FC = () => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Sparkles className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <h2 className="mb-1 font-semibold text-xl">Agent Soul</h2>
          <p className="text-muted-foreground text-sm">
            your Sup's personality, tone, and behavioral rules. The soul evolves
            as your Sup learns how you like to interact.
          </p>
        </div>
      </div>
    </div>
  )
}
