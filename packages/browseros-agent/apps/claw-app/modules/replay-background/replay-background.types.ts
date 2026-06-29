/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared shapes between the background service worker, the
 * recorder content script, and the deriver helpers. Kept in a
 * dedicated types file so the background's setInterval / chrome.*
 * code does not have to share a module with pure-function helpers
 * (which we want to bun-test in isolation).
 */

/**
 * Mirror of the cockpit's TabGroupColor enum. Kept inline so the
 * extension code does not have to import server-side types (the
 * server is a separate package; an import would couple bundle
 * graphs). Updates here must follow updates in
 * `apps/claw-server/src/lib/agent-tab-groups/group-color.ts`.
 */
export type TabGroupColor =
  | 'grey'
  | 'blue'
  | 'red'
  | 'yellow'
  | 'green'
  | 'pink'
  | 'purple'
  | 'cyan'
  | 'orange'

/**
 * One row of `GET /replay/tabs`. The cockpit's `/replay/tabs`
 * service emits this shape; the extension's background worker
 * polls + parses it.
 */
export interface ReplayTab {
  sessionId: string
  tabPageId: number
  url: string
  title: string
  groupColor: TabGroupColor | null
}

export interface ReplayTabsResponse {
  tabs: ReplayTab[]
}

/**
 * What the background worker remembers per chrome tab id. The
 * sessionId is what the recorder POSTs against; tabPageId is the
 * CDP page id baked into every NDJSON line.
 */
export interface ChromeTabRecord {
  sessionId: string
  tabPageId: number
}

/**
 * The diff result emitted when /replay/tabs returns a new snapshot.
 * Driven entirely by the cockpit; the background applies these
 * outcomes via chrome.* APIs.
 */
export interface ReplayMapDiff {
  /** New `(chromeTabId -> record)` entries to inject + remember. */
  added: Array<{ chromeTabId: number; record: ChromeTabRecord }>
  /** Existing entries that disappeared (session ended / tab closed
   *  from the cockpit's view). Background should send `recorder-stop`
   *  to the content script and drop the entry locally. */
  removed: number[]
}

/** Messages content script <-> background. */
export type RecorderMessage =
  | { type: 'recorder-hello' }
  | { type: 'recorder-config'; sessionId: string; tabPageId: number }
  | { type: 'recorder-not-yet' }
  | { type: 'recorder-stop' }
