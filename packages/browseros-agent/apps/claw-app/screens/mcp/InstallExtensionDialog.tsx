import { ArrowLeft, ArrowRight, Check, Download } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button, buttonVariants } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  COWORK_REQUIREMENT_LINE,
  EXTENSION_DOWNLOAD_URL,
  EXTENSION_RELEASES_URL,
  INSTALL_STEPS,
  MCPB_FILENAME,
} from './install-guide.data'

interface InstallExtensionDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * Walks through installing the BrowserOS neo extension in Claude Desktop,
 * one step at a time, with a screenshot of the real control at each stop.
 *
 * `DialogContent` hardcodes `sm:max-w-md`, and a base-only `max-w` is
 * silently ignored at ≥640px, so the width override has to set the `sm:`
 * variant too (same trap as ScreenshotLightbox).
 */
export function InstallExtensionDialog({
  open,
  onOpenChange,
}: InstallExtensionDialogProps) {
  const [index, setIndex] = useState(0)
  const [failedImages, setFailedImages] = useState<ReadonlySet<string>>(
    new Set(),
  )

  useEffect(() => {
    if (open) track(AnalyticsEvent.InstallGuideOpened)
    // Reopening always restarts the walkthrough.
    else setIndex(0)
  }, [open])

  const lastIndex = INSTALL_STEPS.length - 1
  const step = INSTALL_STEPS[Math.min(index, lastIndex)]
  const isLast = index >= lastIndex

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[92vh] w-[min(58rem,94vw)] max-w-[94vw] gap-0 overflow-y-auto p-0 sm:max-w-[min(58rem,94vw)]">
        <div className="flex flex-col sm:flex-row">
          <nav className="shrink-0 border-border-2 border-b bg-bg-sunken px-5 py-5 sm:w-60 sm:border-r sm:border-b-0">
            <DialogTitle className="font-semibold text-[15px] text-ink leading-snug">
              Install BrowserOS neo
            </DialogTitle>
            <DialogDescription className="mt-1 text-[12px] text-ink-3 leading-snug">
              {COWORK_REQUIREMENT_LINE}
            </DialogDescription>
            <ol className="mt-4 flex gap-1 overflow-x-auto sm:flex-col sm:overflow-x-visible">
              {INSTALL_STEPS.map((railStep, railIndex) => {
                const isActive = railIndex === index
                const isDone = railIndex < index
                return (
                  <li key={railStep.id} className="shrink-0 sm:shrink">
                    <button
                      type="button"
                      onClick={() => setIndex(railIndex)}
                      aria-current={isActive ? 'step' : undefined}
                      className={cn(
                        'flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors',
                        isActive
                          ? 'bg-accent-tint text-accent-ink'
                          : 'text-ink-3 hover:bg-card-tint hover:text-ink-2',
                      )}
                    >
                      <span
                        className={cn(
                          'inline-flex size-5 shrink-0 items-center justify-center rounded-full font-mono text-[10px] tabular-nums',
                          isActive
                            ? 'bg-accent text-white'
                            : isDone
                              ? 'bg-green-tint text-green'
                              : 'border border-border-2 text-ink-3',
                        )}
                      >
                        {isDone ? (
                          <Check aria-hidden className="size-3" />
                        ) : (
                          railIndex + 1
                        )}
                      </span>
                      <span className="truncate text-[13px] leading-snug">
                        {railStep.title}
                      </span>
                    </button>
                  </li>
                )
              })}
            </ol>
          </nav>

          <div className="flex min-w-0 flex-1 flex-col px-6 py-5">
            <p className="pr-10 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]">
              step {index + 1} of {INSTALL_STEPS.length}
            </p>
            <h3 className="mt-1 pr-10 font-semibold text-[17px] text-ink leading-snug">
              {step.title}
            </h3>
            <p className="mt-1.5 text-[13.5px] text-ink-2 leading-relaxed">
              {step.body}
            </p>

            <div
              className={cn(
                // Fixed aspect keeps the modal from jumping between steps; the
                // vh cap keeps it inside short viewports.
                'mt-4 aspect-[4/3] max-h-[46vh] w-full overflow-hidden rounded-xl border border-border-2 bg-card-tint',
                !step.image && 'border-dashed',
              )}
            >
              {step.image ? (
                failedImages.has(step.image.src) ? (
                  <div className="flex h-full items-center justify-center px-6 text-center">
                    <p className="text-[13px] text-ink-3 leading-snug">
                      Screenshot could not be loaded — the steps above still
                      apply.
                    </p>
                  </div>
                ) : (
                  <img
                    src={step.image.src}
                    alt={step.image.alt}
                    className="size-full object-contain"
                    onError={() => {
                      const src = step.image?.src
                      if (src === undefined) return
                      setFailedImages((prev) => new Set(prev).add(src))
                    }}
                  />
                )
              ) : (
                <div className="flex h-full flex-col items-center justify-center gap-4 px-6 text-center">
                  <span className="inline-flex size-14 items-center justify-center rounded-2xl bg-accent-tint text-accent">
                    <Download aria-hidden className="size-6" />
                  </span>
                  <p className="font-mono text-[12px] text-ink-2">
                    {MCPB_FILENAME}
                  </p>
                  <a
                    href={EXTENSION_DOWNLOAD_URL}
                    download={MCPB_FILENAME}
                    target="_blank"
                    rel="noreferrer"
                    onClick={() =>
                      track(AnalyticsEvent.InstallGuideDownloadClicked)
                    }
                    className={cn(buttonVariants({ size: 'lg' }), 'px-4')}
                  >
                    <Download aria-hidden />
                    Download the extension
                  </a>
                  <a
                    href={EXTENSION_RELEASES_URL}
                    target="_blank"
                    rel="noreferrer"
                    className="font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em] underline-offset-4 transition-colors hover:text-accent hover:underline"
                  >
                    all releases
                  </a>
                </div>
              )}
            </div>

            <div className="mt-4 flex items-center justify-end gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={index === 0}
                onClick={() => setIndex((prev) => Math.max(0, prev - 1))}
              >
                <ArrowLeft aria-hidden />
                Back
              </Button>
              {isLast ? (
                <Button size="sm" onClick={() => onOpenChange(false)}>
                  <Check aria-hidden />
                  Done
                </Button>
              ) : (
                <Button
                  size="sm"
                  onClick={() =>
                    setIndex((prev) => Math.min(lastIndex, prev + 1))
                  }
                >
                  Next
                  <ArrowRight aria-hidden />
                </Button>
              )}
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
