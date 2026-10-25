import { Blocks } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import { ManualSetupDialog } from './ManualSetupDialog'

interface ManualSetupCardProps {
  /** Endpoint URL from the page, passed through to the dialog. */
  endpointUrl: string | null
}

/**
 * The way in for an agent that is not listed. It sits last on purpose:
 * someone who scanned the rows without finding theirs reads this next.
 */
export function ManualSetupCard({ endpointUrl }: ManualSetupCardProps) {
  const [open, setOpen] = useState(false)

  return (
    <>
      <Card
        size="compact"
        className="flex-row flex-wrap items-center gap-4 border-border-2 bg-card px-[22px] py-5 shadow-card"
      >
        <span
          aria-hidden
          className="grid size-11 shrink-0 place-items-center rounded-full bg-accent-tint text-primary"
        >
          <Blocks className="size-5" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="font-bold text-[16px] text-cyanotype-ink">
            Using another agent?
          </span>
          <span className="text-[14.5px] text-ink-2">
            If yours is not listed, you can connect it by hand. It takes two
            copy and paste steps.
          </span>
        </span>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => setOpen(true)}
          className="font-semibold"
        >
          Set it up by hand
        </Button>
      </Card>
      <ManualSetupDialog
        open={open}
        onOpenChange={setOpen}
        endpointUrl={endpointUrl}
      />
    </>
  )
}
