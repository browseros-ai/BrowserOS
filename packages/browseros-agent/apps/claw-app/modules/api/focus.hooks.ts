/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Activates the exact Chrome tab selected on a live-session card, then
 * focuses the owning window returned by Chrome. A stale tab id rejects
 * through the mutation error path so the caller can report the failure.
 */

import { createMutation } from 'react-query-kit'

export interface FocusBrowserTabResult {
  browserTabId: number
  windowId?: number
}

export interface FocusBrowserTabVariables {
  browserTabId: number
}

/** Brings a tab to the front of its window and that window to the front. */
export async function focusBrowserTab(
  browserTabId: number,
): Promise<FocusBrowserTabResult> {
  const tab = await chrome.tabs.update(browserTabId, { active: true })
  if (!tab) throw new Error(`Chrome tab ${browserTabId} is unavailable`)
  if (typeof tab.windowId === 'number') {
    await chrome.windows.update(tab.windowId, { focused: true })
  }
  return {
    browserTabId,
    windowId: tab.windowId,
  }
}

export const useFocusBrowserTab = createMutation<
  FocusBrowserTabResult,
  FocusBrowserTabVariables
>({
  mutationFn: ({ browserTabId }) => focusBrowserTab(browserTabId),
})
