/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentSettings } from '@browseros/claw-api'
import { JevTokenSource } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { useRef, useState, useSyncExternalStore } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { isFlagEnabled, subscribeToFlags } from '@/modules/analytics/posthog'
import {
  useAgentSettings,
  useRemoveJevToken,
  useSaveJevToken,
  useUpdateAgentSettings,
} from '@/modules/api/agent-settings.hooks'

const HUMAN_HELP_SWITCH_ID = 'human-help-switch'
const HUMAN_HELP_TITLE_ID = 'human-help-title'
const HUMAN_HELP_DESCRIPTION_ID = 'human-help-description'
const CLEANUP_TIP_SWITCH_ID = 'task-cleanup-tip-switch'
const CLEANUP_TIP_TITLE_ID = 'task-cleanup-tip-title'
const CLEANUP_TIP_DESCRIPTION_ID = 'task-cleanup-tip-description'
/**
 * The flag that reveals the token field.
 *
 * Hidden unless PostHog says otherwise for this install, which is also what an
 * install with analytics turned off gets: no evaluation, so no field.
 */
const TOKEN_FIELD_FLAG = 'ultrafast-token-field'
const JEV_INPUT_ID = 'jev-token-input'
const JEV_TITLE_ID = 'jev-token-title'
const JEV_DESCRIPTION_ID = 'jev-token-description'

/**
 * Server-owned agent settings. Switches show the server's saved values.
 */
export function Settings() {
  const settings = useAgentSettings()
  // false as the server snapshot, so a static render and a cold open agree:
  // the field was never there, rather than appearing a beat late.
  const tokenFieldEnabled = useSyncExternalStore(
    subscribeToFlags,
    () => isFlagEnabled(TOKEN_FIELD_FLAG),
    () => false,
  )
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

  const handleCleanupTipChange = (taskCleanupTipEnabled: boolean) => {
    updateSettings.mutate(
      { taskCleanupTipEnabled },
      {
        onSuccess: (saved) => {
          queryClient.setQueryData(useAgentSettings.getKey(), saved)
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
          Choose the tools and tips your agents receive.
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
          <label
            htmlFor={CLEANUP_TIP_SWITCH_ID}
            className="flex items-center justify-between gap-6 border-ledger-divider border-t px-5 py-4"
          >
            <span className="flex flex-col gap-1">
              <span
                id={CLEANUP_TIP_TITLE_ID}
                className="font-medium text-ink text-sm"
              >
                Include tab group cleanup tip
              </span>
              <span
                id={CLEANUP_TIP_DESCRIPTION_ID}
                className="text-[13px] text-ink-2 leading-snug"
              >
                When you finish this task, close your task’s tab group.
              </span>
            </span>
            <Switch
              id={CLEANUP_TIP_SWITCH_ID}
              aria-labelledby={CLEANUP_TIP_TITLE_ID}
              aria-describedby={CLEANUP_TIP_DESCRIPTION_ID}
              checked={settings.data?.taskCleanupTipEnabled ?? false}
              onCheckedChange={handleCleanupTipChange}
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
          {tokenFieldEnabled && (
            <JevToken
              source={settings.data?.jevTokenSource ?? JevTokenSource.None}
            />
          )}
        </div>
      )}
    </div>
  )
}

/**
 * The decision model credential, which is the one thing that decides whether
 * agents are offered `pursue` at all.
 *
 * Three states rather than two: a saved token, a token in the server's
 * environment, or neither. Naming the environment case is what stops a user
 * pasting a token they did not need.
 */
function JevToken({ source }: { source: JevTokenSource }) {
  // Uncontrolled on purpose: the credential lives in the field the user typed
  // it into and is read once on submit, rather than being mirrored into React
  // state and re-rendered.
  const field = useRef<HTMLInputElement>(null)
  const [replacing, setReplacing] = useState(false)
  const save = useSaveJevToken()
  const remove = useRemoveJevToken()
  const queryClient = useQueryClient()

  const publish = (saved: AgentSettings) => {
    queryClient.setQueryData(useAgentSettings.getKey(), saved)
    if (field.current) field.current.value = ''
    setReplacing(false)
  }

  const saved = source === JevTokenSource.Settings
  const showField = !saved || replacing
  const busy = save.isPending || remove.isPending

  return (
    <div className="border-ledger-divider border-t px-5 py-4">
      <div className="flex flex-col gap-1">
        <span id={JEV_TITLE_ID} className="font-medium text-ink text-sm">
          Ultrafast decision loop
        </span>
        <span
          id={JEV_DESCRIPTION_ID}
          className="text-[13px] text-ink-2 leading-snug"
        >
          {saved
            ? 'A token is saved, so agents are offered pursue, which decides each step with a typed model instead of a prompt.'
            : source === JevTokenSource.Environment
              ? 'Using the token in the server’s environment. A token saved here takes precedence over it.'
              : 'Paste a token to offer agents pursue, which decides each step with a typed model instead of a prompt. Without one the tool is not offered at all.'}
        </span>
      </div>

      {showField ? (
        <form
          className="mt-3 flex items-start gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            const token = field.current?.value.trim()
            if (!token) return
            save.mutate({ token }, { onSuccess: publish })
          }}
        >
          <Input
            ref={field}
            id={JEV_INPUT_ID}
            type="password"
            autoComplete="off"
            spellCheck={false}
            aria-labelledby={JEV_TITLE_ID}
            aria-describedby={JEV_DESCRIPTION_ID}
            placeholder="Paste your token"
            disabled={busy}
          />
          <Button type="submit" disabled={busy}>
            Save
          </Button>
          {saved && (
            <Button
              type="button"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                if (field.current) field.current.value = ''
                setReplacing(false)
              }}
            >
              Cancel
            </Button>
          )}
        </form>
      ) : (
        <div className="mt-3 flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={busy}
            onClick={() => setReplacing(true)}
          >
            Replace
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={busy}
            onClick={() => remove.mutate(undefined, { onSuccess: publish })}
          >
            Remove
          </Button>
        </div>
      )}

      <p className="mt-2 text-[12px] text-ink-3 leading-snug">
        Agents pick up the change on their next tool list. An already installed
        skill follows after BrowserOS neo restarts.
      </p>

      {(save.isError || remove.isError) && (
        <p role="alert" className="mt-2 text-[13px] text-destructive">
          Could not save that. Try again.
        </p>
      )}
    </div>
  )
}
