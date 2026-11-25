/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import {
  type QueryClient,
  skipToken,
  useQueryClient,
} from '@tanstack/react-query'
import { Trash2 } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { MorphingModal } from '@/components/motion/morphing-modal'
import {
  useForgetThisComputer,
  usePlan,
  usePlanDevTools,
  usePlanKey,
  usePlanPricing,
} from '@/modules/api/plan.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import {
  DEVELOPMENT_BUILD,
  forgetPlanMemory,
} from './developer-settings.helpers'
import { SettingRow, SettingsSection } from './settings.components'

/**
 * Tools for testing billing, last in Settings. Shown only in a development build talking to a
 * debug server: release servers do not have the routes, so the probe reads false and nothing
 * shows, not even while it loads.
 */
export function DeveloperSettings({
  developmentBuild = DEVELOPMENT_BUILD,
}: {
  developmentBuild?: boolean
}) {
  const tools = usePlanDevTools({
    variables: developmentBuild ? undefined : skipToken,
  })
  const [confirming, setConfirming] = useState(false)
  if (tools.data !== true) return null
  return (
    <SettingsSection title="Developer">
      <SettingRow
        id="forget-this-computer"
        title={
          <span className="inline-flex flex-wrap items-center gap-2">
            Remove licence from this computer
            <span className="rounded-full bg-amber-tint px-2 py-0.5 font-semibold text-[12px] text-amber-ink">
              Debug build
            </span>
          </span>
        }
        description="This computer goes back to Free and gets a new install id, so you can test the free trial again. Your key, its other computers and the subscription don’t change."
        control={
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirming(true)}
            className="font-semibold text-red-ink"
          >
            <Trash2 aria-hidden className="size-4" />
            Remove
          </Button>
        }
      />
      <ForgetDialog open={confirming} onClose={() => setConfirming(false)} />
    </SettingsSection>
  )
}

/**
 * What happens after the removal. The page reloads on success: notes the app keeps in memory for
 * this page load (a skill reminder already seen, a bar already closed) would otherwise stay
 * hidden though their saved copies are gone. A failure leaves the page as it is.
 */
export function forgetCallbacks(
  queryClient: QueryClient,
  onClose: () => void,
  reload: () => void = () => window.location.reload(),
) {
  return {
    onSuccess: (plan: Plan) => {
      queryClient.setQueryData(usePlan.getKey(), plan)
      queryClient.removeQueries({ queryKey: usePlanPricing.getKey() })
      queryClient.removeQueries({ queryKey: usePlanKey.getKey() })
      forgetPlanMemory()
      toastSuccess('Removed. This computer is on Free now.')
      onClose()
      reload()
    },
    onError: (error: Error) =>
      toastApiError('Couldn’t remove the licence from this computer', error),
  }
}

function ForgetDialog({
  open,
  onClose,
}: {
  open: boolean
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const forget = useForgetThisComputer(forgetCallbacks(queryClient, onClose))
  return (
    <MorphingModal
      viewId={open ? 'forget-this-computer' : null}
      onClose={onClose}
      placement="center"
      className="max-w-[440px] border-border-2 bg-card"
    >
      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-2">
          <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
            Remove licence from this computer?
          </h2>
          <p className="text-[14.5px] text-ink-2">
            This computer goes back to Free and gets a new install id, so you
            can test the free trial again.
          </p>
          <p className="text-[14.5px] text-ink-2">
            Your key, its other computers and the subscription don’t change. To
            turn ultrafast back on, use your key again.
          </p>
          <p className="text-[13.5px] text-ink-3">Only in debug builds.</p>
        </div>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" disabled={forget.isPending} onClick={onClose}>
            Keep it
          </Button>
          <StatefulButton
            state={forget.isPending ? 'loading' : 'idle'}
            loadingText="Remove"
            disabled={forget.isPending}
            onClick={() => forget.mutate()}
            className="font-semibold"
          >
            Remove
          </StatefulButton>
        </div>
      </div>
    </MorphingModal>
  )
}
