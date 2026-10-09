/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Background side of the in-page takeover bar. Content scripts on every tab poll
 * this; one short-lived cache coalesces those polls into a single live-sessions
 * read, and resolve goes out from here (the background holds host permissions,
 * so it avoids the page-origin CORS the content script would hit).
 */

import { SessionStatus, type SessionSummary } from '@browseros/claw-api'
import { ClawApiClient } from '@browseros/claw-api-client'
import { takeoverForTab } from './live-help'
import type { TakeoverPollResponse } from './takeover.types'

const CACHE_TTL_MS = 1200

export interface TakeoverBridgeOptions {
  resolveServerBaseUrl: () => Promise<string>
  now?: () => number
}

export interface TakeoverBridge {
  pollForTab: (tabId: number) => Promise<TakeoverPollResponse>
  resolve: (sessionId: string, note?: string) => Promise<void>
}

export function createTakeoverBridge(
  options: TakeoverBridgeOptions,
): TakeoverBridge {
  const now = options.now ?? (() => Date.now())
  let cache: { at: number; sessions: SessionSummary[] } | null = null
  let inflight: Promise<SessionSummary[]> | null = null

  const client = async () =>
    new ClawApiClient(await options.resolveServerBaseUrl(), { fetch })

  const liveSessions = async (): Promise<SessionSummary[]> => {
    if (cache && now() - cache.at < CACHE_TTL_MS) return cache.sessions
    if (inflight) return inflight
    inflight = (async () => {
      try {
        const list = await (await client()).listSessions({
          status: SessionStatus.Live,
        })
        cache = { at: now(), sessions: list.items }
        return list.items
      } catch {
        // A stopped or unreachable server just means no bar; keep the last good
        // read if we have one so a transient blip does not flap the bar.
        return cache?.sessions ?? []
      } finally {
        inflight = null
      }
    })()
    return inflight
  }

  return {
    async pollForTab(tabId) {
      const context = takeoverForTab(await liveSessions(), tabId)
      return context ? { active: true, context } : { active: false }
    },
    async resolve(sessionId, note) {
      await (await client()).resolveHelp({ sessionId, note })
      // Drop the cache so the next poll reflects the resolve and the bar clears.
      cache = null
    },
  }
}
