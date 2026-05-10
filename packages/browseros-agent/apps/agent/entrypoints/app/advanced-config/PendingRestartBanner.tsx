import { AlertTriangle } from 'lucide-react'
import type { FC } from 'react'

interface PendingRestartBannerProps {
  visible: boolean
}

export const PendingRestartBanner: FC<PendingRestartBannerProps> = ({
  visible,
}) => {
  if (!visible) return null

  return (
    <div className="flex items-start gap-3 rounded-xl border border-amber-500/30 bg-amber-500/10 p-4 text-amber-900 dark:text-amber-200">
      <AlertTriangle className="mt-0.5 size-5 shrink-0" />
      <div className="space-y-1">
        <p className="font-semibold text-sm">
          Changes saved. Please quit and reopen BrowserOS to apply.
        </p>
        <p className="text-xs opacity-80">
          Advanced config is loaded at startup only. No live reload.
        </p>
      </div>
    </div>
  )
}
