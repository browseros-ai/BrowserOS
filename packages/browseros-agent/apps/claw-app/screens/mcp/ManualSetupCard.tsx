import { ArrowRight, Terminal } from 'lucide-react'
import { useState } from 'react'
import { MANUAL_SETUP_CARD_LINE } from './install-guide.data'
import { ManualSetupDialog } from './ManualSetupDialog'

interface ManualSetupCardProps {
  /** Endpoint URL from the page, passed through to the dialog. */
  endpointUrl: string | null
}

/**
 * The route for an agent that is not in the Connected agents list.
 *
 * Sits last on purpose. Someone who has just scanned the supported rows
 * without finding their agent needs the next thing they read to be for
 * them, and a catch-all after the named products is where that lands.
 */
export function ManualSetupCard({ endpointUrl }: ManualSetupCardProps) {
  const [open, setOpen] = useState(false)

  return (
    <section className="space-y-2">
      <header className="flex items-baseline justify-between gap-3">
        <h2 className="font-semibold text-cyanotype-ink text-lg">
          Any other agent
        </h2>
        <span className="text-[12px] text-cyanotype-muted">Manual</span>
      </header>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="group block w-full rounded-9 border border-cyanotype-border bg-card p-4 text-left transition-colors hover:border-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
      >
        <div className="flex items-start gap-3">
          <span className="inline-flex size-7 shrink-0 items-center justify-center rounded-lg bg-card-tint text-cyanotype-muted">
            <Terminal aria-hidden className="size-4" />
          </span>
          <div className="min-w-0 flex-1 space-y-1">
            <p className="font-semibold text-[15px] text-cyanotype-ink leading-snug">
              Not on the list above?
            </p>
            <p className="text-[13px] text-cyanotype-muted leading-snug">
              {MANUAL_SETUP_CARD_LINE}
            </p>
          </div>
        </div>
        <div className="mt-3 flex justify-end">
          <span className="inline-flex items-center gap-1 text-[12px] text-cyanotype-blue transition-colors group-hover:text-cyanotype-blue-hover">
            Show me how
            <ArrowRight
              aria-hidden
              className="size-3.5 transition-transform group-hover:translate-x-0.5"
            />
          </span>
        </div>
      </button>
      <ManualSetupDialog
        open={open}
        onOpenChange={setOpen}
        endpointUrl={endpointUrl}
      />
    </section>
  )
}
