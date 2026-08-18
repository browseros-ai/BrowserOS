import { IconChevronLeft, IconChevronRight, IconX } from '@tabler/icons-react'
import { type KeyboardEventHandler, useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Carousel,
  type CarouselApi,
  CarouselContent,
  CarouselItem,
} from '@/components/ui/carousel'
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

export interface ScreenshotLightboxItem {
  screenshotId: number
  sourceUrl: string | null
  offsetMs: number | null
}

interface ScreenshotLightboxProps {
  sessionId: string
  items: readonly ScreenshotLightboxItem[]
  startId: number | null
  onClose: () => void
}

/**
 * Full-size screenshot inspector. Every screenshot in the session is a
 * slide in a swipeable carousel, opened on the clicked one, so the user
 * moves through them all without closing and reopening. A caption + close
 * toolbar sits above the image (never over it), and each image is bounded
 * to the viewport with object-contain so it renders as large as possible
 * without overflow or distortion.
 *
 * DialogContent's default width clamp is `sm:max-w-md` (448px); the
 * `sm:max-w-[94vw]` override is load-bearing, a base-only `max-w` is
 * silently ignored at every width ≥640px.
 */
export function ScreenshotLightbox({
  sessionId,
  items,
  startId,
  onClose,
}: ScreenshotLightboxProps) {
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const open = startId !== null
  // If the clicked screenshot has dropped out of the polled list (e.g. pruned
  // mid-view), keep showing it solo rather than silently jumping to another one.
  const slides =
    startId !== null && !items.some((item) => item.screenshotId === startId)
      ? [{ screenshotId: startId, sourceUrl: null, offsetMs: null }]
      : items
  const startIndex =
    startId === null
      ? 0
      : Math.max(
          0,
          slides.findIndex((item) => item.screenshotId === startId),
        )

  const [api, setApi] = useState<CarouselApi>()
  const [current, setCurrent] = useState(startIndex)
  // Reset the active slide to the clicked one when the lightbox reopens on a
  // different screenshot (state persists across opens); derived in render, not
  // an effect. The Carousel's `key` remounts embla to the same start index.
  const [seenStartId, setSeenStartId] = useState(startId)
  if (startId !== seenStartId) {
    setSeenStartId(startId)
    setCurrent(startIndex)
  }

  // embla is an external system: subscribe so swipes/drags feed back into the
  // caption + counter. Kept in sync on select and on layout re-init.
  useEffect(() => {
    if (!api) return
    const sync = () => setCurrent(api.selectedScrollSnap())
    sync()
    api.on('select', sync)
    api.on('reInit', sync)
    return () => {
      api.off('select', sync)
      api.off('reInit', sync)
    }
  }, [api])

  const canPrev = current > 0
  const canNext = current < slides.length - 1
  const active = slides[current] ?? null
  const host = hostOf(active?.sourceUrl ?? null)
  const caption =
    [
      host,
      active?.offsetMs != null ? `T+${formatOffset(active.offsetMs)}` : null,
    ]
      .filter(Boolean)
      .join(' · ') || 'Screenshot'

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
        'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="combobox"], [role="slider"], [role="spinbutton"], [data-slot="carousel"]',
      )
    ) {
      return
    }

    if (event.key === 'ArrowLeft') {
      if (!canPrev) return
      event.preventDefault()
      api?.scrollPrev()
    } else if (event.key === 'ArrowRight') {
      if (!canNext) return
      event.preventDefault()
      api?.scrollNext()
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        showCloseButton={false}
        className="ph-no-capture flex max-h-[92vh] w-[92vw] max-w-[94vw] flex-col gap-2 bg-transparent p-0 shadow-none ring-0 sm:max-w-[94vw]"
        onKeyDown={handleKeyDown}
      >
        <DialogTitle className="sr-only">Screenshot preview</DialogTitle>
        {open && (
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
                  disabled={!canPrev}
                  className="text-ink-2 hover:bg-card-tint hover:text-ink"
                  onClick={() => api?.scrollPrev()}
                >
                  <IconChevronLeft aria-hidden />
                </Button>
                <span className="min-w-[7ch] text-center font-mono text-[11.5px] text-ink-3 tabular-nums">
                  {slides.length ? current + 1 : 0} / {slides.length}
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label="Next screenshot"
                  disabled={!canNext}
                  className="text-ink-2 hover:bg-card-tint hover:text-ink"
                  onClick={() => api?.scrollNext()}
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
              <Carousel
                key={startId ?? 'closed'}
                setApi={setApi}
                opts={{ startIndex }}
                className="w-full"
              >
                <CarouselContent className="ml-0">
                  {slides.map((item, index) => {
                    const itemHost = hostOf(item.sourceUrl)
                    return (
                      <CarouselItem
                        key={item.screenshotId}
                        className="flex items-center justify-center pl-0"
                      >
                        <img
                          src={taskScreenshotUrl(
                            sessionId,
                            item.screenshotId,
                            screenshotBaseUrl,
                          )}
                          alt={
                            itemHost
                              ? `Screenshot of ${itemHost}`
                              : 'Screenshot'
                          }
                          loading={
                            Math.abs(index - current) <= 1 ? 'eager' : 'lazy'
                          }
                          className="max-h-[calc(92vh-3.5rem)] w-auto max-w-full rounded-xl object-contain shadow-2xl ring-1 ring-foreground/10"
                        />
                      </CarouselItem>
                    )
                  })}
                </CarouselContent>
              </Carousel>
            ) : (
              <div className="aspect-[16/10] max-h-[calc(92vh-3.5rem)] w-full max-w-[94vw] animate-pulse rounded-xl bg-card-tint ring-1 ring-foreground/10" />
            )}
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
