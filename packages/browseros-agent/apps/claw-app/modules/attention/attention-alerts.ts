/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The only place that touches the browser to get the person's attention: the
 * system notification, the dock bounce, and the toolbar badge.
 */

import { badgeText, type OpenHelp } from './attention.helpers'

export const HELP_NOTIFICATION_BUTTONS = {
  helpNow: 0,
  stopTask: 1,
} as const

export const TEST_NOTIFICATION_ID = 'claw-attention-test'

// The amber the app uses for "waiting for you".
const BADGE_COLOR = '#B45309'

export interface AttentionAlerts {
  show: (help: OpenHelp) => Promise<void>
  clear: (requestId: string, browserTabId: number) => Promise<void>
  setCount: (count: number) => Promise<void>
}

function iconUrl(): string {
  return chrome.runtime.getURL('icon/128.png')
}

async function windowOfTab(browserTabId: number): Promise<number | null> {
  try {
    const tab = await chrome.tabs.get(browserTabId)
    return typeof tab.windowId === 'number' ? tab.windowId : null
  } catch {
    return null
  }
}

/**
 * BrowserOS's own attention API, when this build has it: a dock bounce that
 * keeps going until the person looks, and a count on the dock icon. Older
 * builds fall back to the single bounce from `drawAttention`.
 */
function nativeAttention(): typeof chrome.browserOS | null {
  const api = typeof chrome === 'undefined' ? undefined : chrome.browserOS
  return typeof api?.requestAttention === 'function' ? api : null
}

function callNative(run: (done: () => void) => void): Promise<void> {
  return new Promise((resolve) => {
    try {
      run(() => {
        // Reading lastError keeps Chrome from logging an unchecked error.
        void chrome.runtime.lastError
        resolve()
      })
    } catch {
      resolve()
    }
  })
}

/** Bounces the dock icon (macOS) or flashes the taskbar for a tab's window. */
async function drawAttention(browserTabId: number, on: boolean): Promise<void> {
  const windowId = await windowOfTab(browserTabId)
  if (windowId === null) return
  try {
    await chrome.windows.update(windowId, { drawAttention: on })
  } catch {}
}

export function createAttentionAlerts(): AttentionAlerts {
  return {
    async show(help) {
      try {
        await chrome.notifications.create(help.requestId, {
          type: 'basic',
          iconUrl: iconUrl(),
          title: help.title,
          message: help.message,
          buttons: [{ title: 'Help now' }, { title: 'Stop task' }],
          requireInteraction: true,
          priority: 2,
        })
      } catch {}
      const native = nativeAttention()
      if (native) {
        await callNative((done) =>
          native.requestAttention({ critical: true }, done),
        )
      } else {
        await drawAttention(help.browserTabId, true)
      }
    },
    async clear(requestId, browserTabId) {
      try {
        await chrome.notifications.clear(requestId)
      } catch {}
      await drawAttention(browserTabId, false)
    },
    async setCount(count) {
      const native = nativeAttention()
      if (native) {
        await callNative((done) => native.setDockBadge(badgeText(count), done))
        if (count === 0)
          await callNative((done) => native.cancelAttention(done))
      }
      try {
        await chrome.action.setBadgeBackgroundColor({ color: BADGE_COLOR })
        await chrome.action.setBadgeTextColor?.({ color: '#FFFFFF' })
        await chrome.action.setBadgeText({ text: badgeText(count) })
        await chrome.action.setTitle({
          title:
            count === 0
              ? 'BrowserOS neo'
              : `${count} ${count === 1 ? 'agent needs' : 'agents need'} you`,
        })
      } catch {}
    },
  }
}

/** A sample notification, so the person can see what an alert looks like. */
export async function showTestNotification(): Promise<void> {
  await chrome.notifications.create(TEST_NOTIFICATION_ID, {
    type: 'basic',
    iconUrl: iconUrl(),
    title: 'Your AI agent needs you',
    message:
      'This is how neo tells you an agent is stuck on a sign-in, a code or a captcha.',
    priority: 2,
  })
}

/** Whether the system lets the browser show notifications at all. */
export function notificationPermission(): Promise<'granted' | 'denied'> {
  return new Promise((resolve) => {
    try {
      chrome.notifications.getPermissionLevel((level) => resolve(level))
    } catch {
      // No notification API to ask means nothing to warn about.
      resolve('granted')
    }
  })
}
