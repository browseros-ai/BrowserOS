/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Background wiring for help alerts: a periodic look at the live sessions, the
 * notification buttons, and the watcher that both feed.
 */

import type { SessionSummary } from '@browseros/claw-api'
import { ClawApiClient } from '@browseros/claw-api-client'
import { focusBrowserTab } from '@/modules/api/focus.hooks'
import { readNotifyPreference } from './attention.preferences'
import {
  createAttentionAlerts,
  HELP_NOTIFICATION_BUTTONS,
  TEST_NOTIFICATION_ID,
} from './attention-alerts'
import { createHelpStream } from './help-stream'
import { chromeAnnouncedStore, createHelpWatcher } from './help-watcher'

const WATCH_ALARM = 'claw-help-watch'
// Chrome's shortest repeating alarm. Page polls and the help stream cover the
// time in between; this is what still notices with every page closed.
const WATCH_PERIOD_MINUTES = 0.5

export interface AttentionOptions {
  resolveServerBaseUrl: () => Promise<string>
  /** Reads the live sessions; `fresh` skips the takeover bridge's cache. */
  liveSessions: (fresh?: boolean) => Promise<SessionSummary[]>
}

export interface Attention {
  /** Feeds a fresh read of the live sessions from anywhere in the worker. */
  sync: (sessions: readonly SessionSummary[]) => void
  /** Looks right now, outside the alarm's rhythm. */
  check: () => void
}

export function registerAttention(options: AttentionOptions): Attention {
  const alerts = createAttentionAlerts()
  const watcher = createHelpWatcher({
    alerts,
    store: chromeAnnouncedStore(),
    notifyEnabled: readNotifyPreference,
  })
  const client = async () =>
    new ClawApiClient(await options.resolveServerBaseUrl(), { fetch })

  const check = (fresh = false) => {
    void options
      .liveSessions(fresh)
      .then((sessions) => watcher.sync(sessions))
      .catch(() => {})
  }

  // The server says the moment a help request changes; the alarm is the
  // fallback while the stream is down, and it also wakes a sleeping worker to
  // reopen the stream.
  const stream = createHelpStream({
    resolveServerBaseUrl: options.resolveServerBaseUrl,
    onChange: () => check(true),
  })
  stream.ensure()

  void chrome.alarms.create(WATCH_ALARM, {
    periodInMinutes: WATCH_PERIOD_MINUTES,
  })
  chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name !== WATCH_ALARM) return
    stream.ensure()
    check()
  })

  const helpNow = async (requestId: string) => {
    const help = await watcher.lookup(requestId)
    await chrome.notifications.clear(requestId).catch(() => {})
    if (!help) return
    // The takeover bar on that tab takes it from here, Hand back included.
    await focusBrowserTab(help.browserTabId).catch(() => {})
  }

  const stopTask = async (requestId: string) => {
    const help = await watcher.lookup(requestId)
    await chrome.notifications.clear(requestId).catch(() => {})
    if (!help) return
    await (await client())
      .cancelSession({ sessionId: help.sessionId })
      .catch(() => {})
    check()
  }

  chrome.notifications.onClicked.addListener((notificationId) => {
    if (notificationId === TEST_NOTIFICATION_ID) {
      void chrome.notifications.clear(notificationId)
      return
    }
    void helpNow(notificationId)
  })
  chrome.notifications.onButtonClicked.addListener(
    (notificationId, buttonIndex) => {
      if (buttonIndex === HELP_NOTIFICATION_BUTTONS.helpNow) {
        void helpNow(notificationId)
      } else if (buttonIndex === HELP_NOTIFICATION_BUTTONS.stopTask) {
        void stopTask(notificationId)
      }
    },
  )

  check()
  return {
    sync: (sessions) => {
      void watcher.sync(sessions)
    },
    check: () => check(true),
  }
}
