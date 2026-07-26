import { Sparkles } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'

export interface Props {
  onStart: () => void
  isLoading?: boolean
}

export const Welcome: FC<Props> = ({ onStart, isLoading }) => {
  return (
    <div className="rounded-xl border border-border bg-card p-8 text-center shadow-sm">
      <div className="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-full bg-[var(--accent-orange)]/10">
        <Sparkles className="h-8 w-8 text-[var(--accent-orange)]" />
      </div>
      <h3 className="mb-2 font-semibold text-lg">Share Your Experience</h3>
      <p className="mx-auto mb-6 max-w-md text-muted-foreground text-sm">
        We'd love to learn about how you use browser automation. This quick
        conversation helps us build features that matter to you.
      </p>
      <Button onClick={onStart} disabled={isLoading} size="lg">
        {isLoading ? 'Starting...' : 'Start Survey'}
      </Button>
    </div>
  )
}
