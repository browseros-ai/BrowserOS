import { MessageSquareHeart } from 'lucide-react'
import type { FC } from 'react'

export const Header: FC = () => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <MessageSquareHeart className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <h2 className="mb-1 font-semibold text-xl">Product Survey</h2>
          <p className="text-muted-foreground text-sm">
            We'd love your honest feedback. All responses are anonymous.
          </p>
        </div>
      </div>
    </div>
  )
}
