/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ClawApiClient } from '@browseros/claw-api-client'
import type { Fetcher } from './recordings-relay'

export interface SessionReplaysSettingOptions {
  resolveServerBaseUrl: () => Promise<string>
  fetch?: Fetcher
  now?: () => number
}

export interface SessionReplaysSetting {
  /** Whether a newly loaded page should start recording. */
  enabled: () => Promise<boolean>
}

const SETTING_CACHE_TTL_MS = 30_000

/**
 * The person's "Record task replays" choice as the background answers it to
 * each new page. Cached briefly so a page load does not wait on the server.
 * An unreachable server keeps recording, as before the setting existed; the
 * server drops batches itself while replays are off, so a stale answer never
 * stores a replay.
 */
export function createSessionReplaysSetting(
  options: SessionReplaysSettingOptions,
): SessionReplaysSetting {
  const fetch = options.fetch ?? globalThis.fetch
  const now = options.now ?? Date.now
  let cached: { enabled: boolean; expiresAt: number } | null = null

  return {
    async enabled() {
      if (cached && now() < cached.expiresAt) return cached.enabled
      try {
        const client = new ClawApiClient(await options.resolveServerBaseUrl(), {
          fetch,
        })
        const settings = await client.getAgentSettings()
        const enabled = settings.sessionReplaysEnabled !== false
        cached = { enabled, expiresAt: now() + SETTING_CACHE_TTL_MS }
        return enabled
      } catch {
        return true
      }
    },
  }
}
