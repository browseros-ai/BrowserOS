/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * "Notify me when an agent needs me" is a choice about this device, not about
 * the agents, so it lives in extension storage rather than on the server.
 */

const NOTIFY_KEY = 'claw:attention:notify'

export async function readNotifyPreference(): Promise<boolean> {
  const stored = await chrome.storage.local.get(NOTIFY_KEY)
  return stored[NOTIFY_KEY] !== false
}

export async function writeNotifyPreference(
  enabled: boolean,
): Promise<boolean> {
  await chrome.storage.local.set({ [NOTIFY_KEY]: enabled })
  return enabled
}
