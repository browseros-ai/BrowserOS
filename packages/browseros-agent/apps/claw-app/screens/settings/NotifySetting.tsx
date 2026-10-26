import { useQueryClient } from '@tanstack/react-query'
import { BellRing } from 'lucide-react'
import { Button } from '@/components/motion/button/base'
import { Switch } from '@/components/motion/switch'
import {
  useNotificationPermission,
  useNotifyPreference,
  useSetNotifyPreference,
} from '@/modules/attention/attention.hooks'
import { showTestNotification } from '@/modules/attention/attention-alerts'
import { toastApiError } from '@/modules/toast/toast'
import { SettingRow } from './settings.components'

const NOTIFY_ID = 'notify-help'

/**
 * Whether neo reaches you outside the browser when an agent is stuck: a
 * system notification and a bounce of the dock icon.
 */
export function NotifySetting() {
  const queryClient = useQueryClient()
  const preference = useNotifyPreference()
  const permission = useNotificationPermission()
  const setPreference = useSetNotifyPreference({
    onSuccess: (enabled) => {
      queryClient.setQueryData(useNotifyPreference.getKey(), enabled)
    },
    onError: (error) => toastApiError('Couldn’t save that setting', error),
  })
  const enabled = preference.data ?? true

  return (
    <SettingRow
      id={NOTIFY_ID}
      title="Notify me when an agent needs me"
      description="A notification and a bounce of the BrowserOS icon in your dock, even when BrowserOS is in the background."
      control={
        <Switch
          ariaLabel="Notify me when an agent needs me"
          ariaDescribedBy={`${NOTIFY_ID}-description`}
          checked={enabled}
          onCheckedChange={(next) => setPreference.mutate(next)}
          disabled={!preference.data && preference.isPending}
        />
      }
    >
      {enabled ? (
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="secondary"
            size="sm"
            onClick={() =>
              showTestNotification().catch((error) =>
                toastApiError('Couldn’t show a notification', error),
              )
            }
            className="font-semibold"
          >
            <BellRing aria-hidden className="size-4" />
            Send a test notification
          </Button>
          {permission.data === 'denied' ? (
            <p role="status" className="text-[13.5px] text-amber-ink">
              Your computer is blocking notifications from BrowserOS. Turn them
              on in System Settings, Notifications, BrowserOS.
            </p>
          ) : (
            <p className="text-[13.5px] text-ink-3">
              Pick Alerts for BrowserOS in System Settings, Notifications to
              keep them on screen until you act.
            </p>
          )}
        </div>
      ) : null}
    </SettingRow>
  )
}
