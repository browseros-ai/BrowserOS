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
  /** Existing entries whose sessionId / tabPageId changed for the
   *  same chrome tab id (a tab that survived a session swap, or the
   *  cockpit reassigned it). Background should send
   *  `recorder-restart` so the live content script tears down the
   *  old rrweb recorder and re-initialises with the new config. We
   *  do NOT re-inject via `chrome.scripting` here because the
   *  content script's `__browserosClawReplayInstalled` guard
   *  short-circuits subsequent injects within the same document. */
  changed: Array<{ chromeTabId: number; record: ChromeTabRecord }>
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
  | {
      /**
       * Cockpit reassigned this chrome tab id to a different session
       * (or different tabPageId). Content script tears down its
       * current rrweb recorder and re-initialises with this new
       * config without going through another `recorder-hello`
       * round-trip. The install guard stays set; only the recorder
       * state inside the script is swapped.
       */
      type: 'recorder-restart'
      sessionId: string
      tabPageId: number
    }
  | {
      /**
       * Content script forwards an NDJSON batch of rrweb events to
       * the background, which POSTs them to the cockpit. Direct
       * page-side fetch to http://127.0.0.1:9200 is blocked by
       * Chrome's Private Network Access policy when the document
       * origin is public (HTTPS); the background's fetch runs in
       * the extension's chrome-extension:// origin and is not
       * subject to PNA.
       */
      type: 'recorder-events'
      sessionId: string
      ndjson: string
    }
