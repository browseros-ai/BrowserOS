import { ArrowLeft, ArrowRight, Check, Download } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button, ButtonLink } from '@/components/motion/button/base'
import { MorphingModal } from '@/components/motion/morphing-modal'
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
 * Walks through installing the BrowserOS neo extension in the Claude desktop
 * app, one step at a time, with a picture of the real control at each stop.
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
    <MorphingModal
      // One view for the whole guide: a view per step swapped the rail and
      // resized the panel on every step, which shook the dialog.
      viewId={open ? 'guide' : null}
      onClose={() => onOpenChange(false)}
      placement="center"
      ariaLabel="Set up the Claude desktop app"
      className="max-h-[calc(100dvh-32px)] max-w-[58rem] overflow-y-auto rounded-[22px] border-border-2 bg-card"
    >
      <div className="-m-5 flex flex-col sm:flex-row">
        <nav className="shrink-0 border-border-2 border-b bg-bg-sunken px-5 py-5 sm:w-60 sm:border-r sm:border-b-0">
          <h2 className="font-bold text-[16px] text-cyanotype-ink leading-snug">
            Set up the Claude desktop app
          </h2>
          <p className="mt-1 text-[12.5px] text-ink-3 leading-snug">
            {COWORK_REQUIREMENT_LINE}
          </p>
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
          {/* Room for the longest step, so the picture below never moves. */}
          <p className="mt-1.5 min-h-[3lh] text-[13.5px] text-ink-2 leading-relaxed">
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
                    Couldn’t load this picture. The steps above still apply.
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
                <ButtonLink
                  href={EXTENSION_DOWNLOAD_URL}
                  download={MCPB_FILENAME}
                  target="_blank"
                  rel="noreferrer"
                  onClick={() =>
                    track(AnalyticsEvent.InstallGuideDownloadClicked)
                  }
                  className="font-semibold"
                >
                  <Download aria-hidden className="size-4" />
                  Download the extension
                </ButtonLink>
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
              variant="ghost"
              size="sm"
              disabled={index === 0}
              onClick={() => setIndex((prev) => Math.max(0, prev - 1))}
              className="font-semibold"
            >
              <ArrowLeft aria-hidden className="size-4" />
              Back
            </Button>
            {isLast ? (
              <Button
                size="sm"
                onClick={() => onOpenChange(false)}
                className="font-semibold"
              >
                <Check aria-hidden className="size-4" />
                Done
              </Button>
            ) : (
              <Button
                size="sm"
                onClick={() =>
                  setIndex((prev) => Math.min(lastIndex, prev + 1))
                }
                className="font-semibold"
              >
                Next
                <ArrowRight aria-hidden className="size-4" />
              </Button>
            )}
          </div>
        </div>
      </div>
    </MorphingModal>
  )
}
