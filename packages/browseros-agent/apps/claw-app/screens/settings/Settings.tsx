/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentSettings } from '@browseros/claw-api'
import { JevTokenSource } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { Monitor, Moon, Sun } from 'lucide-react'
import { useRef, useState, useSyncExternalStore } from 'react'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Switch } from '@/components/motion/switch'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { useTheme } from '@/components/theme-provider'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { normalizeTheme } from '@/lib/theme/theme-storage'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { isFlagEnabled, subscribeToFlags } from '@/modules/analytics/posthog'
import {
  useAgentSettings,
  useRemoveJevToken,
  useSaveJevToken,
  useUpdateAgentSettings,
} from '@/modules/api/agent-settings.hooks'
import { SettingRow, SettingsSection } from './settings.components'

const HUMAN_HELP_ID = 'human-help'
const CLEANUP_TIP_ID = 'task-cleanup-tip'
const THEME_ID = 'theme'
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

/** Every setting in one place, each in plain words. */
export function Settings() {
  const settings = useAgentSettings()
  // false as the server snapshot, so a static render and a cold open agree:
  // the field was never there, rather than appearing a beat late.
  const tokenFieldEnabled = useSyncExternalStore(
    subscribeToFlags,
    () => isFlagEnabled(TOKEN_FIELD_FLAG),
    () => false,
  )

  return (
    <Page>
      <PageHeader title="Settings" />
      <AgentSwitches />
      <Appearance />
      {tokenFieldEnabled && settings.data ? (
        <SettingsSection title="Advanced">
          <JevToken
            source={settings.data.jevTokenSource ?? JevTokenSource.None}
          />
        </SettingsSection>
      ) : null}
    </Page>
  )
}

// ---------------------------------------------------------------------------

/** Server-owned switches that change what agents are told while they work. */
function AgentSwitches() {
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

  if (settings.isError) {
    return (
      <SettingsSection title="While your AI works">
        <p role="alert" className="px-5 py-4 text-[14.5px] text-ink-2">
          Couldn’t load these settings. Check that BrowserOS is running, then
          open this page again.
        </p>
      </SettingsSection>
    )
  }

  const busy = !settings.data || updateSettings.isPending

  return (
    <SettingsSection title="While your AI works">
      <SettingRow
        id={HUMAN_HELP_ID}
        title="Ask me when it gets stuck"
        description="neo pauses and asks you to sign in, enter a code, or solve a captcha. When this is off, your AI agents ask you in their own chat instead."
        control={
          <Switch
            ariaLabel="Ask me when it gets stuck"
            ariaDescribedBy={`${HUMAN_HELP_ID}-description`}
            // The server's default (on) until it answers, so the switch
            // does not flip on load unless the user turned it off.
            checked={settings.data?.humanHelpEnabled ?? true}
            onCheckedChange={handleHumanHelpChange}
            disabled={busy}
          />
        }
      />
      <SettingRow
        id={CLEANUP_TIP_ID}
        title="Tidy up when a task finishes"
        description="Your AI agents close the pages they opened once they’re done."
        control={
          <Switch
            ariaLabel="Tidy up when a task finishes"
            ariaDescribedBy={`${CLEANUP_TIP_ID}-description`}
            checked={settings.data?.taskCleanupTipEnabled ?? false}
            onCheckedChange={handleCleanupTipChange}
            disabled={busy}
          />
        }
      >
        {updateSettings.isError ? (
          <p role="alert" className="text-[13.5px] text-red-ink">
            Couldn’t save that setting. Try again.
          </p>
        ) : null}
      </SettingRow>
    </SettingsSection>
  )
}

const THEME_OPTIONS = [
  { value: 'light', label: 'Light', Icon: Sun },
  { value: 'dark', label: 'Dark', Icon: Moon },
  { value: 'system', label: 'Match browser', Icon: Monitor },
] as const

/** Light, dark, or whatever BrowserOS itself is set to. */
function Appearance() {
  const { theme, setTheme } = useTheme()
  return (
    <SettingsSection title="Appearance">
      <SettingRow
        id={THEME_ID}
        title="Theme"
        description="Match browser follows your BrowserOS appearance setting."
        control={
          <Tabs
            variant="segment"
            value={theme}
            onValueChange={(value) => setTheme(normalizeTheme(value))}
          >
            <TabsList
              aria-label="Theme"
              aria-describedby={`${THEME_ID}-description`}
              className="border border-border-2"
            >
              {THEME_OPTIONS.map(({ value, label, Icon }) => (
                <TabsTrigger
                  key={value}
                  value={value}
                  icon={<Icon className="size-4" />}
                  className="px-3 font-semibold text-[13.5px]"
                >
                  {label}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
        }
      />
    </SettingsSection>
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
