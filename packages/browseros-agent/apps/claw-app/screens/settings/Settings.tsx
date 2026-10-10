/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Switch } from '@/components/ui/switch'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  useAgentSettings,
  useUpdateAgentSettings,
} from '@/modules/api/agent-settings.hooks'

/** Server-owned agent settings. Changes apply to the next tool list an agent reads. */
export function Settings() {
  const { data, isError } = useAgentSettings()
  const update = useUpdateAgentSettings()

  const handleHumanHelpChange = (enabled: boolean) => {
    update.mutate(
      { humanHelpEnabled: enabled },
      {
        onSuccess: () => track(AnalyticsEvent.HumanHelpToggled, { enabled }),
      },
    )
  }

  return (
    <div className="relative mx-auto flex w-full max-w-3xl flex-col gap-6 px-8 pt-8 pb-16">
      <header className="flex flex-col gap-1">
        <h1 className="font-extrabold text-3xl leading-tight tracking-tight md:text-4xl">
          Settings
        </h1>
        <p className="text-ink-2 text-sm">
          Choose which BrowserOS neo tools your agents can use.
        </p>
      </header>

      {isError ? (
        <p className="rounded-9 border border-ledger-border bg-card px-4 py-3 text-ink-2 text-sm">
          Could not load settings. Check that BrowserOS neo is running and try
          again.
        </p>
      ) : (
        <label
          htmlFor="human-help-enabled"
          className="flex items-center justify-between gap-6 rounded-9 border border-ledger-border bg-card px-4 py-4"
        >
          <span className="flex flex-col gap-1">
            <span className="font-medium text-sm">Ask for human help</span>
            <span className="text-ink-2 text-sm">
              Lets agents pause and ask you to take over a page here, for
              sign-ins, codes, or captchas. When off, agents tell you what they
              need in their own chat instead.
            </span>
          </span>
          <Switch
            id="human-help-enabled"
            checked={data?.humanHelpEnabled ?? true}
            onCheckedChange={handleHumanHelpChange}
            disabled={!data || update.isPending}
          />
        </label>
      )}
      {update.isError ? (
        <p className="text-destructive text-sm">
          Could not save this setting. Try again.
        </p>
      ) : null}
    </div>
  )
}
