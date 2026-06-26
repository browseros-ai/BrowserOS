/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Builds the per-session JavaScript blob the cockpit injects into
 * every agent-driven BrowserOS page via CDP
 * `Page.addScriptToEvaluateOnNewDocument`. The blob is one
 * self-contained IIFE that:
 *
 *   1. Inlines the vendored rrweb UMD bundle so the page never has
 *      to fetch a remote script (no CSP friction).
 *   2. Bakes in this session's `sessionId`, the page's `tabPageId`,
 *      and the cockpit's loopback origin.
 *   3. Calls `rrweb.record` with sensible defaults (password fields
 *      masked) and buffers events.
 *   4. POSTs each batch to the cockpit's audit-replay events route
 *      via `fetch({keepalive: true})`, and flushes via
 *      `navigator.sendBeacon` on `pagehide` so unload events are
 *      not dropped.
 *
 * `window.__browserosClawReplayInstalled` guards against double-
 * install when `addScriptToEvaluateOnNewDocument` fires twice for
 * the same document (it always does at least once per nav).
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'

/**
 * Read the vendored UMD bundle at module load time. We use
 * `readFileSync` once on cold start; the contents are cached for the
 * process lifetime so per-tab injection is just a string interpolation.
 *
 * The path is resolved relative to this source file rather than
 * relative to the process cwd so the script is found regardless of
 * which directory the server is launched from.
 */
const RRWEB_UMD_SOURCE = readFileSync(
  join(import.meta.dir, '..', 'vendor', 'rrweb.umd.min.js'),
  'utf8',
)

export interface BuildInitScriptInput {
  sessionId: string
  tabPageId: number
  cockpitOrigin: string
}

export function buildInitScript(input: BuildInitScriptInput): string {
  const { sessionId, tabPageId, cockpitOrigin } = input
  // JSON.stringify handles quote escaping and unicode safely for the
  // user-controllable inputs (sessionId in particular; the rest are
  // server-controlled but defence in depth costs nothing).
  return `(function(){
if (window.__browserosClawReplayInstalled) return;
window.__browserosClawReplayInstalled = true;
${RRWEB_UMD_SOURCE}
;(function(){
  var sessionId = ${JSON.stringify(sessionId)};
  var tabPageId = ${JSON.stringify(tabPageId)};
  var url = ${JSON.stringify(cockpitOrigin)} + '/audit/replay/' + sessionId + '/events';
  if (!window.rrweb || typeof window.rrweb.record !== 'function') return;
  var buf = [];
  var timer = null;
  function send(body){
    try {
      if (typeof navigator.sendBeacon === 'function' && document.visibilityState === 'hidden') {
        navigator.sendBeacon(url, new Blob([body], { type: 'application/x-ndjson' }));
        return;
      }
      fetch(url, {
        method: 'POST',
        headers: { 'content-type': 'application/x-ndjson' },
        body: body,
        keepalive: true,
        credentials: 'omit'
      }).catch(function(){});
    } catch (e) { /* swallow; recorder must never throw into page */ }
  }
  function flush(){
    if (buf.length === 0) return;
    var body = buf.map(function(e){
      return JSON.stringify({ tabPageId: tabPageId, ts: e.ts, type: e.type, data: e.data });
    }).join('\\n');
    buf = [];
    send(body);
  }
  window.rrweb.record({
    maskInputOptions: { password: true },
    emit: function(event){
      buf.push({
        ts: (event && typeof event.timestamp === 'number') ? event.timestamp : Date.now(),
        type: event && event.type,
        data: event && event.data
      });
      if (buf.length >= 50) {
        flush();
      } else if (timer === null) {
        timer = setTimeout(function(){ timer = null; flush(); }, 2000);
      }
    }
  });
  window.addEventListener('pagehide', flush);
  window.addEventListener('visibilitychange', function(){
    if (document.visibilityState === 'hidden') flush();
  });
})();
})();`
}

/** Test-only accessor so we can assert the vendor file is loadable. */
export function _rrwebVendorSizeForTesting(): number {
  return RRWEB_UMD_SOURCE.length
}
