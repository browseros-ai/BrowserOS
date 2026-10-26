/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Turns snapshots of the live sessions into alerts: each new help request is
 * announced once, each finished one is cleared, and the count stays current.
 * What was announced is kept outside the worker, so a restarted background
 * never announces the same request twice.
 */

import type { SessionSummary } from '@browseros/claw-api'
import { diffHelp, openHelpRequests } from './attention.helpers'
import type { AttentionAlerts } from './attention-alerts'

/** What the watcher remembers about a request it announced. */
export interface AnnouncedHelp {
  requestId: string
  sessionId: string
  browserTabId: number
}

export interface AnnouncedStore {
  load: () => Promise<AnnouncedHelp[]>
  save: (announced: AnnouncedHelp[]) => Promise<void>
}

export interface HelpWatcherOptions {
  alerts: AttentionAlerts
  store: AnnouncedStore
}

export interface HelpWatcher {
  /** Applies one snapshot of the live sessions. */
  sync: (sessions: readonly SessionSummary[]) => Promise<void>
  /** The request behind a notification, for its buttons. */
  lookup: (requestId: string) => Promise<AnnouncedHelp | undefined>
}

export function createHelpWatcher(options: HelpWatcherOptions): HelpWatcher {
  const { alerts, store } = options
  // Snapshots arrive from the alarm and from page polls at once; one at a time
  // keeps a request from being announced twice.
  let queue: Promise<void> = Promise.resolve()

  const apply = async (sessions: readonly SessionSummary[]) => {
    const open = openHelpRequests(sessions)
    const announced = await store.load()
    const { started, ended } = diffHelp(
      announced.map((help) => help.requestId),
      open,
    )
    for (const help of started) await alerts.show(help)
    for (const requestId of ended) {
      const gone = announced.find((help) => help.requestId === requestId)
      if (gone) await alerts.clear(requestId, gone.browserTabId)
    }
    await alerts.setCount(open.length)
    await store.save(
      open.map(({ requestId, sessionId, browserTabId }) => ({
        requestId,
        sessionId,
        browserTabId,
      })),
    )
  }

  return {
    sync(sessions) {
      queue = queue.then(() => apply(sessions)).catch(() => {})
      return queue
    },
    async lookup(requestId) {
      return (await store.load()).find((help) => help.requestId === requestId)
    },
  }
}

const ANNOUNCED_KEY = 'claw:attention:announced'

/** Session storage: survives the worker sleeping, not the browser quitting. */
export function chromeAnnouncedStore(): AnnouncedStore {
  return {
    async load() {
      const stored = await chrome.storage.session.get(ANNOUNCED_KEY)
      const value = stored[ANNOUNCED_KEY]
      return Array.isArray(value) ? (value as AnnouncedHelp[]) : []
    },
    async save(announced) {
      await chrome.storage.session.set({ [ANNOUNCED_KEY]: announced })
    },
  }
}
