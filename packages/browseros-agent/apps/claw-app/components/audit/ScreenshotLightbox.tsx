import { IconChevronLeft, IconChevronRight, IconX } from '@tabler/icons-react'
import type { KeyboardEventHandler } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { formatOffset, hostOf } from './screenshot.helpers'

interface ScreenshotLightboxProps {
  sessionId: string
  screenshotId: number | null
  screenshotIds: readonly number[]
  sourceUrl?: string | null
  offsetMs?: number | null
  onNavigate: (screenshotId: number) => void
  onClose: () => void
}

/**
 * Full-size screenshot inspector. A caption + close toolbar sits above
 * the image (never over it), and the image is bounded to the viewport
 * with object-contain so it renders as large as possible without
 * overflow or distortion.
 *
 * DialogContent's default width clamp is `sm:max-w-md` (448px); the
 * `sm:max-w-[94vw]` override is load-bearing — a base-only `max-w` is
 * silently ignored at every width ≥640px.
 */
export function ScreenshotLightbox({
  sessionId,
  screenshotId,
  screenshotIds,
  sourceUrl = null,
  offsetMs = null,
  onNavigate,
  onClose,
}: ScreenshotLightboxProps) {
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const host = hostOf(sourceUrl)
  const caption =
    [host, offsetMs != null ? `T+${formatOffset(offsetMs)}` : null]
      .filter(Boolean)
      .join(' · ') || 'Screenshot'
  const orderedScreenshotIds =
    screenshotId !== null && !screenshotIds.includes(screenshotId)
      ? [screenshotId]
      : screenshotIds
  const activeIndex =
    screenshotId === null ? -1 : orderedScreenshotIds.indexOf(screenshotId)
  const previousId =
    activeIndex > 0 ? (orderedScreenshotIds[activeIndex - 1] ?? null) : null
  const nextId =
    activeIndex >= 0 && activeIndex < orderedScreenshotIds.length - 1
      ? (orderedScreenshotIds[activeIndex + 1] ?? null)
      : null

  const handleKeyDown: KeyboardEventHandler<HTMLDivElement> = (event) => {
    if (
      event.nativeEvent.isComposing ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey
    ) {
      return
    }

    const target = event.target
    if (
      target instanceof Element &&
      target.closest(
        'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="combobox"], [role="slider"], [role="spinbutton"]',
      )
    ) {
      return
    }

    const destinationId =
      event.key === 'ArrowLeft'
        ? previousId
        : event.key === 'ArrowRight'
          ? nextId
          : null
    if (destinationId === null) return

    event.preventDefault()
    onNavigate(destinationId)
  }

  return (
    <Dialog open={screenshotId !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        showCloseButton={false}
        className="ph-no-capture flex max-h-[92vh] w-auto max-w-[94vw] flex-col gap-2 bg-transparent p-0 shadow-none ring-0 sm:max-w-[94vw]"
        onKeyDown={handleKeyDown}
      >
        <DialogTitle className="sr-only">Screenshot preview</DialogTitle>
        {screenshotId !== null && (
          <>
            <div className="flex items-center gap-3 rounded-lg bg-popover/95 px-3 py-2 ring-1 ring-foreground/10 supports-backdrop-filter:backdrop-blur">
              <span className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-ink-2">
                {caption}
              </span>
              <div className="flex shrink-0 items-center gap-1">
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label="Previous screenshot"
                  disabled={previousId === null}
                  className="text-ink-2 hover:bg-card-tint hover:text-ink"
                  onClick={() => previousId !== null && onNavigate(previousId)}
                >
                  <IconChevronLeft aria-hidden />
                </Button>
                <span className="min-w-[7ch] text-center font-mono text-[11.5px] text-ink-3 tabular-nums">
                  {activeIndex + 1} / {orderedScreenshotIds.length}
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label="Next screenshot"
                  disabled={nextId === null}
                  className="text-ink-2 hover:bg-card-tint hover:text-ink"
                  onClick={() => nextId !== null && onNavigate(nextId)}
                >
                  <IconChevronRight aria-hidden />
                </Button>
                <span aria-hidden className="mx-1 h-4 w-px bg-border-2" />
                <DialogClose
                  render={
                    <Button type="button" variant="ghost" size="icon-sm" />
                  }
                >
                  <IconX aria-hidden />
                  <span className="sr-only">Close</span>
                </DialogClose>
              </div>
            </div>
            {screenshotBaseUrl !== null ? (
              <img
                src={taskScreenshotUrl(
                  sessionId,
                  screenshotId,
                  screenshotBaseUrl,
                )}
                alt={host ? `Screenshot of ${host}` : 'Screenshot'}
                className="max-h-[calc(92vh-3.5rem)] w-auto max-w-[94vw] rounded-xl object-contain shadow-2xl ring-1 ring-foreground/10"
              />
            ) : (
              <div className="aspect-[16/10] max-h-[calc(92vh-3.5rem)] w-[70vw] max-w-[94vw] animate-pulse rounded-xl bg-card-tint ring-1 ring-foreground/10" />
            )}
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
