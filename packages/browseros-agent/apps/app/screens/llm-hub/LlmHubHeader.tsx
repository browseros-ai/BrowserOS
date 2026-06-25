import { MessageSquare } from 'lucide-react'
import type { FC } from 'react'

export const LlmHubHeader: FC = () => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <MessageSquare className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div>
          <h2 className="mb-1 font-semibold text-xl">LLM Chat & Hub</h2>
          <p className="text-muted-foreground text-sm">
            Curate and configure your favourite LLM chat providers
          </p>
        </div>
      </div>
    </div>
  )
}
