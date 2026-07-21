import { IconX } from '@tabler/icons-react'
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
  sourceUrl?: string | null
  offsetMs?: number | null
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
  sourceUrl = null,
  offsetMs = null,
  onClose,
}: ScreenshotLightboxProps) {
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const host = hostOf(sourceUrl)
  const caption =
    [host, offsetMs != null ? `T+${formatOffset(offsetMs)}` : null]
      .filter(Boolean)
      .join(' · ') || 'Screenshot'

  return (
    <Dialog open={screenshotId !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        showCloseButton={false}
        className="flex max-h-[92vh] w-auto max-w-[94vw] flex-col gap-2 bg-transparent p-0 shadow-none ring-0 sm:max-w-[94vw]"
      >
        <DialogTitle className="sr-only">Screenshot preview</DialogTitle>
        {screenshotId !== null && (
          <>
            <div className="flex items-center justify-between gap-3 rounded-lg bg-popover/95 px-3 py-2 ring-1 ring-foreground/10 supports-backdrop-filter:backdrop-blur">
              <span className="min-w-0 truncate font-mono text-[12.5px] text-ink-2">
                {caption}
              </span>
              <DialogClose render={<Button variant="ghost" size="icon-sm" />}>
                <IconX />
                <span className="sr-only">Close</span>
              </DialogClose>
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
