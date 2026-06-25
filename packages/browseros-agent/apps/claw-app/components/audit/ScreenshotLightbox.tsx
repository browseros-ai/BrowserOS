import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { taskScreenshotUrl } from '@/modules/api/audit.hooks'

interface ScreenshotLightboxProps {
  dispatchId: number | null
  onClose: () => void
}

export function ScreenshotLightbox({
  dispatchId,
  onClose,
}: ScreenshotLightboxProps) {
  return (
    <Dialog open={dispatchId !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-4xl border-none bg-transparent p-0 shadow-none">
        <DialogTitle className="sr-only">Screenshot</DialogTitle>
        {dispatchId !== null && (
          <img
            src={taskScreenshotUrl(dispatchId)}
            alt={`Screenshot from dispatch ${dispatchId}`}
            className="h-auto w-full rounded-lg"
          />
        )}
      </DialogContent>
    </Dialog>
  )
}
