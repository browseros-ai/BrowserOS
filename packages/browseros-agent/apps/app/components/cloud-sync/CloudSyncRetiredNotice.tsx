import { HardDrive, X } from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import { cloudSyncNoticeDismissedStorage } from '@/lib/cloud-sync/cloud-sync-storage'

/**
 * Tells the user what changed, once, wherever their synced data used to live.
 *
 * Deliberately past tense. Sync stops in the same release this ships, so a
 * warning about the future would be describing something that has already
 * happened. It also answers the question people will actually have, which is
 * not whether sync is going away but whether they are about to lose anything.
 *
 * Dismissal persists: this is a one-time announcement, not a standing banner,
 * and it should not reappear on every visit to settings.
 */
export const CloudSyncRetiredNotice: FC = () => {
  const [visible, setVisible] = useState(false)

  // Reading persisted dismissal is an async read from extension storage, so
  // the banner starts hidden and appears only once we know it was not dismissed.
  useEffect(() => {
    let cancelled = false
    cloudSyncNoticeDismissedStorage.getValue().then((dismissed) => {
      if (!cancelled) setVisible(!dismissed)
    })
    return () => {
      cancelled = true
    }
  }, [])

  if (!visible) return null

  const dismiss = () => {
    setVisible(false)
    void cloudSyncNoticeDismissedStorage.setValue(true)
  }

  return (
    <div className="flex items-center gap-4 rounded-xl border border-border bg-card p-4 shadow-sm">
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10">
        <HardDrive className="h-5 w-5 text-[var(--accent-orange)]" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="font-semibold text-sm">
          Your data now stays on this device
        </p>
        <p className="text-muted-foreground text-xs">
          Cloud sync has been turned off. Your providers, agents and schedules
          are stored on this machine and keep working. Chats saved to the cloud
          stay visible in history for now.
        </p>
      </div>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss"
        className="shrink-0 rounded-sm p-1 text-muted-foreground opacity-50 transition-opacity hover:opacity-100"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  )
}
