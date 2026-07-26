/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Sidebar entry for the privacy / analytics opt-out. Opens a small
 * dialog explaining what is (and is not) collected and a switch bound
 * to the server-owned consent flag, so one toggle governs both the
 * cockpit and the local server's telemetry.
 */

import { useQueryClient } from '@tanstack/react-query'
import { ShieldCheck } from 'lucide-react'
import { useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Switch } from '@/components/ui/switch'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { applyTelemetry } from '@/modules/analytics/posthog'
import {
  TELEMETRY_QUERY_KEY,
  useSetTelemetryConsent,
  useTelemetryState,
} from '@/modules/analytics/telemetry.hooks'

export interface SidebarPrivacyProps {
  expanded?: boolean
}

export function SidebarPrivacy({ expanded = false }: SidebarPrivacyProps) {
  const [open, setOpen] = useState(false)
  const { data } = useTelemetryState()
  const setConsent = useSetTelemetryConsent()
  const queryClient = useQueryClient()

  const consent = data?.consent ?? false
  const distinctId = data?.distinctId

  const handleChange = (next: boolean) => {
    if (!distinctId) return
    setConsent.mutate(
      { consent: next },
      {
        onSuccess: (state) => {
          queryClient.setQueryData(TELEMETRY_QUERY_KEY, state)
          // Reconcile off the server's authoritative effective state
          // (respects the kill-switch), ordering so the opt-out event
          // still sends while capture is live and the opt-in event sends
          // after init.
          if (state.enabled) {
            applyTelemetry({ distinctId: state.distinctId, enabled: true })
            track(AnalyticsEvent.OptOutToggled, { enabled: true })
          } else {
            track(AnalyticsEvent.OptOutToggled, { enabled: false })
            applyTelemetry({ distinctId: state.distinctId, enabled: false })
          }
        },
      },
    )
  }

  const trigger = (
    <button
      type="button"
      className="flex h-9 w-full items-center gap-3 overflow-hidden whitespace-nowrap rounded-md px-2.5 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
    >
      <ShieldCheck className="size-5 shrink-0" />
      <span
        className={cn(
          'truncate transition-opacity duration-200',
          expanded ? 'opacity-100' : 'opacity-0',
        )}
      >
        Privacy
      </span>
    </button>
  )

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      {expanded ? (
        <DialogTrigger render={trigger} />
      ) : (
        <Tooltip>
          <TooltipTrigger render={<DialogTrigger render={trigger} />} />
          <TooltipContent side="right">Privacy</TooltipContent>
        </Tooltip>
      )}
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Privacy & analytics</DialogTitle>
          <DialogDescription>
            BrowserClaw collects anonymous, aggregate usage (which agents
            connect and which screens you open) to improve the product. It never
            collects the pages you browse, your prompts, tool inputs or outputs,
            or any page content. No account or personal data is used.
          </DialogDescription>
        </DialogHeader>
        <label
          htmlFor="share-anonymous-usage"
          className="flex items-center justify-between gap-4 rounded-lg border border-border px-4 py-3"
        >
          <span className="font-medium text-sm">Share anonymous usage</span>
          <Switch
            id="share-anonymous-usage"
            checked={consent}
            onCheckedChange={handleChange}
            disabled={!distinctId || setConsent.isPending}
          />
        </label>
      </DialogContent>
    </Dialog>
  )
}
