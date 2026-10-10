/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useQueryClient } from '@tanstack/react-query'
import { Switch } from '@/components/ui/switch'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  useAgentSettings,
  useUpdateAgentSettings,
} from '@/modules/api/agent-settings.hooks'

const HUMAN_HELP_SWITCH_ID = 'human-help-switch'
const HUMAN_HELP_TITLE_ID = 'human-help-title'
const HUMAN_HELP_DESCRIPTION_ID = 'human-help-description'

/**
 * Server-owned agent tool settings. The switch shows the server's value, and
 * a change applies to the next tool list an agent reads.
 */
export function Settings() {
  const settings = useAgentSettings()
  const updateSettings = useUpdateAgentSettings()
  const queryClient = useQueryClient()

  const handleHumanHelpChange = (humanHelpEnabled: boolean) => {
    updateSettings.mutate(
      { humanHelpEnabled },
      {
        onSuccess: (saved) => {
          queryClient.setQueryData(useAgentSettings.getKey(), saved)
          track(AnalyticsEvent.HumanHelpToggled, {
            enabled: saved.humanHelpEnabled,
          })
        },
      },
    )
  }

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col gap-8 px-8 pt-8 pb-16">
      <header className="flex flex-col gap-1">
        <h1 className="font-extrabold text-3xl leading-tight tracking-tight md:text-4xl">
          Settings
        </h1>
        <p className="text-ink-2 text-sm">
          Choose which BrowserOS neo tools your agents can use.
        </p>
      </header>

      {settings.isError ? (
        <div className="rounded-9 border border-ledger-border bg-card px-6 py-10 text-center text-ink-2 text-sm">
          Could not load settings. Check that BrowserOS neo is running and try
          again.
        </div>
      ) : (
        <div className="rounded-9 border border-ledger-border bg-card">
          <label
            htmlFor={HUMAN_HELP_SWITCH_ID}
            className="flex items-center justify-between gap-6 px-5 py-4"
          >
            <span className="flex flex-col gap-1">
              <span
                id={HUMAN_HELP_TITLE_ID}
                className="font-medium text-ink text-sm"
              >
                Ask for human help
              </span>
              <span
                id={HUMAN_HELP_DESCRIPTION_ID}
                className="text-[13px] text-ink-2 leading-snug"
              >
                Lets agents pause and ask you to take over the page for a
                sign-in, a code, or a captcha. When off, agents tell you what
                they need in their own chat instead.
              </span>
            </span>
            <Switch
              id={HUMAN_HELP_SWITCH_ID}
              aria-labelledby={HUMAN_HELP_TITLE_ID}
              aria-describedby={HUMAN_HELP_DESCRIPTION_ID}
              // The server's default (on) until it answers, so the switch
              // does not flip on load unless the user turned it off.
              checked={settings.data?.humanHelpEnabled ?? true}
              onCheckedChange={handleHumanHelpChange}
              disabled={!settings.data || updateSettings.isPending}
            />
          </label>
          {updateSettings.isError && (
            <p
              role="alert"
              className="border-ledger-divider border-t px-5 py-3 text-[13px] text-destructive"
            >
              Could not save this setting. Try again.
            </p>
          )}
        </div>
      )}
    </div>
  )
}
