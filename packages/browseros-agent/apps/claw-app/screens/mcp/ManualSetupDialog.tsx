import { ArrowUpRight } from 'lucide-react'
import { useEffect } from 'react'
import { CodeBlock } from '@/components/agents/code-block'
import { Button, ButtonLink } from '@/components/motion/button/base'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { Skeleton } from '@/components/ui/skeleton'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { MANUAL_SETUP_DOCS_URL, MANUAL_SETUP_STEPS } from './install-guide.data'

interface ManualSetupDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Endpoint URL from the page, so step one can show the real value. */
  endpointUrl: string | null
}

/**
 * Connecting an agent that is not listed, by hand. Both steps show at once:
 * they are two copy and paste actions, and seeing them together is the point.
 */
export function ManualSetupDialog({
  open,
  onOpenChange,
  endpointUrl,
}: ManualSetupDialogProps) {
  // Counts opens from any opener, not one call site.
  useEffect(() => {
    if (open) track(AnalyticsEvent.ManualSetupOpened)
  }, [open])

  return (
    <CenterMorphModal open={open} onOpenChange={onOpenChange}>
      <CenterMorphModalContent
        ariaLabel="Connect another agent by hand"
        showCloseButton
        className="max-w-[600px] rounded-[22px] border-border-2 bg-card"
      >
        <div className="flex max-h-[min(760px,calc(100dvh-64px))] flex-col">
          <header className="flex flex-col gap-1 px-6 pt-6 pr-16 pb-4">
            <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              Connect another agent by hand
            </h2>
            <p className="text-[14px] text-ink-2">
              Two copy and paste steps. If you get stuck, the guide has pictures
              for each agent.
            </p>
          </header>

          <ol className="flex min-h-0 flex-col gap-6 overflow-y-auto border-border-2 border-y px-6 py-5">
            {MANUAL_SETUP_STEPS.map((step, index) => (
              <li key={step.id} className="flex gap-3.5">
                <span className="grid size-7 shrink-0 place-items-center rounded-full bg-accent-tint font-semibold text-[13px] text-accent-ink tabular-nums">
                  {index + 1}
                </span>
                <div className="flex min-w-0 flex-1 flex-col gap-2.5">
                  <h3 className="pt-0.5 font-bold text-[15.5px] text-cyanotype-ink">
                    {step.title}
                  </h3>
                  <p className="text-[14px] text-ink-2">{step.body}</p>
                  {step.id === 'add-mcp' ? (
                    endpointUrl ? (
                      <CodeBlock
                        code={endpointUrl}
                        language="text"
                        showLineNumbers={false}
                        copyable
                      />
                    ) : (
                      <Skeleton
                        role="status"
                        aria-label="Finding the address"
                        className="h-11 rounded-xl"
                      />
                    )
                  ) : null}
                  {step.command !== undefined ? (
                    <CodeBlock
                      code={step.command}
                      language="bash"
                      showLineNumbers={false}
                      copyable
                      onCopy={() =>
                        track(AnalyticsEvent.ManualSetupCommandCopied)
                      }
                    />
                  ) : null}
                  {step.note !== undefined ? (
                    <p className="text-[13px] text-ink-3">{step.note}</p>
                  ) : null}
                </div>
              </li>
            ))}
          </ol>

          <footer className="flex items-center justify-between gap-3 px-6 py-4">
            <ButtonLink
              href={MANUAL_SETUP_DOCS_URL}
              target="_blank"
              rel="noreferrer"
              variant="ghost"
              size="sm"
              className="font-semibold"
            >
              Open the guide
              <ArrowUpRight aria-hidden className="size-4" />
            </ButtonLink>
            <CenterMorphModalClose>
              <Button className="font-semibold">Done</Button>
            </CenterMorphModalClose>
          </footer>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
