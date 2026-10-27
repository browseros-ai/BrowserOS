/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { JevTokenSource } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import {
  BellRing,
  ChevronDown,
  Monitor,
  Moon,
  SlidersHorizontal,
  Sun,
} from 'lucide-react'
import { useSyncExternalStore } from 'react'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Button } from '@/components/motion/button/base'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'
import { Switch } from '@/components/motion/switch'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { useTheme } from '@/components/theme-provider'
import { normalizeTheme } from '@/lib/theme/theme-storage'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { isFlagEnabled, subscribeToFlags } from '@/modules/analytics/posthog'
import {
  useAgentSettings,
  useUpdateAgentSettings,
} from '@/modules/api/agent-settings.hooks'
import { useNotificationPermission } from '@/modules/attention/attention.hooks'
import { showTestNotification } from '@/modules/attention/attention-alerts'
import { toastApiError } from '@/modules/toast/toast'
import { HelpSettings } from './HelpSettings'
import { PrivacySettings } from './PrivacySettings'
import { StorageSettings } from './StorageSettings'
import { SettingRow, SettingsSection } from './settings.components'
import { UltrafastKey } from './UltrafastKey'

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
      <StorageSettings />
      <PrivacySettings />
      <HelpSettings />
      {tokenFieldEnabled && settings.data ? (
        <Collapsible className="rounded-2xl border border-border-2 bg-card shadow-card">
          <CollapsibleTrigger className="flex w-full items-center gap-2.5 rounded-2xl px-[18px] py-3.5 text-left font-semibold text-[15px] text-ink-2 outline-none hover:text-cyanotype-ink focus-visible:ring-2 focus-visible:ring-ring">
            <SlidersHorizontal aria-hidden className="size-[15px]" />
            <span className="flex-1">Advanced</span>
            <CollapsibleIndicator>
              <ChevronDown aria-hidden className="size-4" />
            </CollapsibleIndicator>
          </CollapsibleTrigger>
          <CollapsibleContent contentClassName="px-[18px] pb-[18px]">
            <UltrafastKey
              source={settings.data.jevTokenSource ?? JevTokenSource.None}
            />
          </CollapsibleContent>
        </Collapsible>
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
        description="If an agent needs you to sign in, type a code or solve a captcha, it waits and you get a notification. When this is off, agents ask you in their own app instead."
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
      >
        {(settings.data?.humanHelpEnabled ?? true) ? (
          <NotificationCheck />
        ) : null}
      </SettingRow>
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

/**
 * Lets the person see what a notification looks like, which is also when
 * macOS first asks to allow them, and says plainly if the Mac blocks them.
 */
function NotificationCheck() {
  const permission = useNotificationPermission()
  return (
    <div className="flex flex-col items-start gap-2">
      <Button
        variant="secondary"
        size="sm"
        onClick={() =>
          showTestNotification()
            .catch((error) =>
              toastApiError('Couldn’t show a notification', error),
            )
            .finally(() => void permission.refetch())
        }
        className="font-semibold"
      >
        <BellRing aria-hidden className="size-4" />
        Send a test notification
      </Button>
      {permission.data === 'denied' ? (
        <p role="status" className="text-[13.5px] text-amber-ink">
          Your Mac is blocking notifications from BrowserOS. To allow them, open
          System Settings, click Notifications, then BrowserOS.
        </p>
      ) : null}
    </div>
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
                  className="font-semibold text-[13.5px]"
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
