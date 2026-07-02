/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Background service worker for the BrowserClaw extension.
 *
 * Owns the cockpit -> chrome tab-id bridge for the rrweb session
 * replay recorder. The worker:
 *
 *   1. Polls the resolved local cockpit `/replay/tabs` every 2 seconds
 *      for the cockpit's list of currently-agent-driven CDP tabs.
 *   2. For each row, resolves the corresponding `chrome.tabs.id`
 *      via `chrome.tabs.query({url})` narrowed by
 *      `chrome.tabGroups.query({})` colour matching.
 *   3. Injects `recorder.content.js` into newly-discovered tabs
 *      via `chrome.scripting.executeScript`. Operator-owned tabs
 *      that are not in the cockpit's list are never touched.
 *   4. Re-injects on every `chrome.webNavigation.onCommitted` for
 *      tabs in the map (each new document needs its own script
 *      context).
 *   5. Responds to the content script's `recorder-hello` message
 *      with the matching `{sessionId, tabPageId}`.
 *   6. Sends `recorder-stop` to content scripts when the cockpit
 *      removes their tab from the live list.
 *
 * The worker is MV3-recyclable; Chrome may recycle the service
 * worker at any time. State is rebuilt from scratch on next poll;
 * no persistence needed.
 */

import { defineBackground } from 'wxt/utils/define-background'
import { resolveBrowserOSServerBaseUrl } from '@/modules/api/browseros-ports'
import {
  type ChromeTabRecord,
  diffReplayMap,
  normalizeUrl,
  pickChromeTab,
  type RecorderMessage,
  type ReplayTab,
  type ReplayTabsResponse,
} from '@/modules/replay-background'

const POLL_INTERVAL_MS = 2_000
// WXT outputs `entrypoints/recorder.content.ts` to this path inside
// the built extension. The leading slash is required by
// chrome.scripting; without it the API treats the value as a
// relative path against an undefined base.
const CONTENT_SCRIPT_PATH = 'content-scripts/recorder.js'

export default defineBackground(() => {
  const map = new Map<number, ChromeTabRecord>()

  async function poll(): Promise<void> {
    let resp: Response
    try {
      const cockpitOrigin = await resolveBrowserOSServerBaseUrl()
      resp = await fetch(`${cockpitOrigin}/replay/tabs`)
    } catch {
      return
    }
    if (!resp.ok) return
    let body: ReplayTabsResponse
    try {
      body = (await resp.json()) as ReplayTabsResponse
    } catch {
      return
    }
    const resolved = await resolveChromeTabIds(body.tabs)
    const diff = diffReplayMap(map, resolved)
    for (const tabId of diff.removed) {
      map.delete(tabId)
      try {
        await chrome.tabs.sendMessage(tabId, {
          type: 'recorder-stop',
        } satisfies RecorderMessage)
      } catch {
        // Tab may have closed; ignore.
      }
    }
    // Same chrome tab id, new sessionId/tabPageId. The content
    // script is still installed (the install guard would block a
    // re-inject), so we cannot use chrome.scripting.executeScript
    // here. The script's onMessage handler swaps recorder state
    // in place.
    for (const entry of diff.changed) {
      map.set(entry.chromeTabId, entry.record)
      try {
        await chrome.tabs.sendMessage(entry.chromeTabId, {
          type: 'recorder-restart',
          sessionId: entry.record.sessionId,
          tabPageId: entry.record.tabPageId,
        } satisfies RecorderMessage)
      } catch (err) {
        // eslint-disable-next-line no-console
        console.warn('[browseros-claw replay] restart message failed', {
          tabId: entry.chromeTabId,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
    for (const entry of diff.added) {
      map.set(entry.chromeTabId, entry.record)
      try {
        await chrome.scripting.executeScript({
          target: { tabId: entry.chromeTabId, allFrames: false },
          files: [CONTENT_SCRIPT_PATH],
        })
      } catch (err) {
        // eslint-disable-next-line no-console
        console.warn('[browseros-claw replay] inject failed', {
          tabId: entry.chromeTabId,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  async function resolveChromeTabIds(
    tabs: ReplayTab[],
  ): Promise<Array<{ chromeTabId: number; record: ChromeTabRecord }>> {
    const groupColors = await readGroupColors()
    // Query all chrome tabs once. chrome.tabs.query({url}) expects a
    // match pattern, not a raw URL; the cockpit's URLs include query
    // strings that contain `?`, `&`, `=` which are not valid pattern
    // syntax, so passing them directly returned zero matches in
    // dogfood. Query-all + post-filter by exact-after-normalize URL
    // is the reliable shape.
    let allTabs: chrome.tabs.Tab[] = []
    try {
      allTabs = await chrome.tabs.query({})
    } catch {
      return []
    }
    const out: Array<{ chromeTabId: number; record: ChromeTabRecord }> = []
    for (const tab of tabs) {
      const targetUrl = normalizeUrl(tab.url)
      const candidates = allTabs.filter(
        (t) => typeof t.url === 'string' && normalizeUrl(t.url) === targetUrl,
      )
      const chromeTabId = pickChromeTab({
        candidates: candidates.map((c) => ({
          id: c.id,
          groupId: c.groupId,
          url: c.url,
          title: c.title,
        })),
        groupColors,
        replayTab: tab,
      })
      if (chromeTabId === null) continue
      out.push({
        chromeTabId,
        record: { sessionId: tab.sessionId, tabPageId: tab.tabPageId },
      })
    }
    return out
  }

  async function readGroupColors(): Promise<Map<number, string>> {
    const out = new Map<number, string>()
    try {
      const groups = await chrome.tabGroups.query({})
      for (const g of groups) {
        if (typeof g.id === 'number' && typeof g.color === 'string') {
          out.set(g.id, g.color)
        }
      }
    } catch {
      // ignore; the picker degrades to first-match.
    }
    return out
  }

  // Re-inject on every navigation for agent-driven tabs. Each new
  // document gets a fresh script context; the prior content script
  // is destroyed when its document goes away.
  chrome.webNavigation.onCommitted.addListener(async (details) => {
    if (details.frameId !== 0) return // main frame only
    if (!map.has(details.tabId)) return
    try {
      await chrome.scripting.executeScript({
        target: { tabId: details.tabId, allFrames: false },
        files: [CONTENT_SCRIPT_PATH],
      })
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('[browseros-claw replay] reinject failed', {
        tabId: details.tabId,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  })

  // Content scripts ask "what is my config" right after injection,
  // and forward NDJSON event batches via 'recorder-events'.
  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    const msg = message as RecorderMessage
    if (msg.type === 'recorder-hello') {
      const tabId = sender.tab?.id
      if (typeof tabId !== 'number') {
        sendResponse({ type: 'recorder-not-yet' } satisfies RecorderMessage)
        return true
      }
      const record = map.get(tabId)
      if (!record) {
        sendResponse({ type: 'recorder-not-yet' } satisfies RecorderMessage)
        return true
      }
      sendResponse({
        type: 'recorder-config',
        sessionId: record.sessionId,
        tabPageId: record.tabPageId,
      } satisfies RecorderMessage)
      return true
    }
    if (msg.type === 'recorder-events') {
      // The cockpit serves HTTP loopback. Content scripts run in
      // the page's origin context; an HTTPS page POSTing to
      // 127.0.0.1 is blocked by Chrome's Private Network Access
      // policy. The background's fetch runs in the extension's
      // chrome-extension:// origin and is exempt.
      void postRecorderEvents(msg)
      return false
    }
    return false
  })

  async function postRecorderEvents(msg: RecorderMessage): Promise<void> {
    if (msg.type !== 'recorder-events') return
    try {
      const cockpitOrigin = await resolveBrowserOSServerBaseUrl()
      await fetch(`${cockpitOrigin}/audit/replay/${msg.sessionId}/events`, {
        method: 'POST',
        headers: { 'content-type': 'application/x-ndjson' },
        body: msg.ndjson,
        credentials: 'omit',
      })
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('[browseros-claw replay] events POST failed', {
        sessionId: msg.sessionId,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  // Best-effort cleanup when the operator closes a recorded tab.
  chrome.tabs.onRemoved.addListener((tabId) => {
    map.delete(tabId)
  })

  // Kick off the polling loop. setInterval is OK in MV3 service
  // workers; if Chrome recycles the worker, the next /replay/tabs
  // poll on restart rebuilds the map from scratch.
  setInterval(() => {
    void poll()
  }, POLL_INTERVAL_MS)
  void poll()
})
