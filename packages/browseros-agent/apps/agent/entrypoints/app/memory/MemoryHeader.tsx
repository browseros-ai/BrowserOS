import { Brain } from 'lucide-react'
import type { FC } from 'react'

export const MemoryHeader: FC = () => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-violet-500/10">
          <Brain className="h-6 w-6 text-violet-500" />
        </div>
        <div className="flex-1">
          <h2 className="mb-1 font-semibold text-xl">Agent Memory</h2>
          <p className="text-muted-foreground text-sm">
            Facts your Sup remembers about you — your name, preferences,
            projects, and tools. Edit directly or teach through conversation.
          </p>
        </div>
      </div>
    </div>
  )
}
