import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import {
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'

interface ScreenshotLightboxProps {
  dispatchId: number | null
  onClose: () => void
}

export function ScreenshotLightbox({
  dispatchId,
  onClose,
}: ScreenshotLightboxProps) {
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  return (
    <Dialog open={dispatchId !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-4xl border-none bg-transparent p-0 shadow-none">
        <DialogTitle className="sr-only">Screenshot</DialogTitle>
        {dispatchId !== null && screenshotBaseUrl !== null ? (
          <img
            src={taskScreenshotUrl(dispatchId, screenshotBaseUrl)}
            alt={`Screenshot from dispatch ${dispatchId}`}
            className="h-auto w-full rounded-lg"
          />
        ) : dispatchId !== null ? (
          <div className="aspect-[16/10] w-full animate-pulse rounded-lg bg-card-tint" />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}
