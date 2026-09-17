import { ArrowUpRight, Check } from 'lucide-react'
import { useEffect } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { CommandBlock } from './CommandBlock'
import { MANUAL_SETUP_DOCS_URL, MANUAL_SETUP_STEPS } from './install-guide.data'

interface ManualSetupDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Endpoint URL from the page, so step one can show the real value. */
  endpointUrl: string | null
}

/**
 * Manual setup for an agent that is not in the Connected agents list.
 *
 * Both steps are shown at once rather than as a walkthrough. The install
 * guide next door is paged because it is six screenshots inside another
 * application; this is two commands, and the point of the dialog is that a
 * reader sees both of them together. Hiding step two behind a Next button
 * would rebuild the exact gap this screen is here to close.
 *
 * `DialogContent` hardcodes `sm:max-w-md` and a base-only `max-w` is ignored
 * at >=640px, so the width override sets the `sm:` variant too.
 */
export function ManualSetupDialog({
  open,
  onOpenChange,
  endpointUrl,
}: ManualSetupDialogProps) {
  useEffect(() => {
    if (open) track(AnalyticsEvent.ManualSetupOpened)
  }, [open])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[92vh] w-[min(38rem,94vw)] max-w-[94vw] overflow-y-auto p-0 sm:max-w-[min(38rem,94vw)]">
        <div className="border-border-2 border-b px-6 pt-5 pb-4">
          <DialogTitle className="pr-10 font-semibold text-[17px] text-ink leading-snug">
            Connect any other agent
          </DialogTitle>
          <DialogDescription className="mt-1.5 text-[13.5px] text-ink-2 leading-relaxed">
            Connecting a listed agent does two things, so doing it by hand means
            doing both. The MCP server lets your agent reach the browser. The
            skill teaches it how to use it.
          </DialogDescription>
        </div>

        <ol className="flex flex-col gap-6 px-6 py-5">
          {MANUAL_SETUP_STEPS.map((step, index) => (
            <li key={step.id} className="flex gap-3.5">
              <span className="inline-flex size-6 shrink-0 items-center justify-center rounded-full bg-accent font-mono text-[11px] text-white tabular-nums">
                {index + 1}
              </span>
              <div className="min-w-0 flex-1 space-y-2.5">
                <h3 className="font-semibold text-[15px] text-ink leading-snug">
                  {step.title}
                </h3>
                <p className="text-[13.5px] text-ink-2 leading-relaxed">
                  {step.body}
                </p>

                {step.id === 'add-mcp' && (
                  <CommandBlock command={endpointUrl} />
                )}

                {step.command !== undefined && (
                  <CommandBlock
                    command={step.command}
                    onCopied={() =>
                      track(AnalyticsEvent.ManualSetupCommandCopied)
                    }
                  />
                )}

                {step.note !== undefined && (
                  <p className="text-[12px] text-ink-3 leading-relaxed">
                    {step.note}
                  </p>
                )}
              </div>
            </li>
          ))}
        </ol>

        <div className="flex items-center justify-between gap-3 border-border-2 border-t px-6 py-4">
          <a
            href={MANUAL_SETUP_DOCS_URL}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 text-[12.5px] text-accent underline-offset-4 transition-colors hover:underline"
          >
            Full setup guide
            <ArrowUpRight aria-hidden className="size-3.5" />
          </a>
          <Button size="sm" onClick={() => onOpenChange(false)}>
            <Check aria-hidden />
            Done
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
