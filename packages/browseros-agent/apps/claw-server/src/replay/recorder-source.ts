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
 *   3. Calls `rrweb.record` with throttled sampling so heavy SPA
 *      pages do not back up the main thread.
 *   4. POSTs each batch to the cockpit's audit-replay events route
 *      via a plain `fetch`, with `navigator.sendBeacon` as the
 *      `pagehide` flush so unload events are not dropped.
 *
 * Throttling decisions (manual dogfood found the v1 emit handler
 * locked heavy SPA pages; this version drops the noisy event
 * categories at source via rrweb's built-in sampling):
 *
 *   - `sampling.mousemove: false` drops MouseMove entirely.
 *   - `sampling.scroll: 250` rate-limits scroll to one event per
 *     250ms.
 *   - `sampling.media: 500` rate-limits media + canvas updates.
 *   - `sampling.input: 'last'` only records the final value of
 *     each input field rather than every keystroke. The audit
 *     replay does not need keystroke timing; it needs final values
 *     for context.
 *   - `recordCanvas: false` skips canvas capture entirely (huge
 *     event payloads and rarely useful for agent replay).
 *   - The emit handler defers JSON.stringify into a microtask so
 *     the rrweb hot path returns to the page as fast as possible.
 *   - A 500-event in-memory cap drops the oldest events with a
 *     console warn if the page produces faster than we can flush;
 *     prevents a stuck flush from blowing up memory.
 *
 * `window.__browserosClawReplayInstalled` guards against double-
 * install when `addScriptToEvaluateOnNewDocument` fires twice for
 * the same document.
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'

/**
 * Read the vendored UMD bundle at module load time. We use
 * `readFileSync` once on cold start; the contents are cached for the
 * process lifetime so per-tab injection is just a string interpolation.
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
  return `(function(){
if (window.__browserosClawReplayInstalled) return;
window.__browserosClawReplayInstalled = true;
${RRWEB_UMD_SOURCE}
;(function(){
  var sessionId = ${JSON.stringify(sessionId)};
  var tabPageId = ${JSON.stringify(tabPageId)};
  var url = ${JSON.stringify(cockpitOrigin)} + '/audit/replay/' + sessionId + '/events';
  if (!window.rrweb || typeof window.rrweb.record !== 'function') return;

  // Buffer of pre-serialised NDJSON lines so the flush hot path is
  // just a join + fetch; serialisation happens off-emit via a
  // microtask scheduled below.
  var buf = [];
  var BUFFER_CAP = 500;
  var FLUSH_INTERVAL_MS = 2500;
  var timer = null;
  var dropped = 0;
  var pendingSerialise = null;

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
        credentials: 'omit'
      }).catch(function(){ /* recorder must never throw into page */ });
    } catch (e) { /* swallow */ }
  }

  function flush(){
    if (buf.length === 0) return;
    var body = buf.join('\\n');
    buf = [];
    if (dropped > 0) {
      try { console.warn('[browseros-claw replay] dropped', dropped, 'events under buffer pressure'); } catch (e) {}
      dropped = 0;
    }
    send(body);
  }

  function armFlushTimer(){
    if (timer !== null) return;
    timer = setTimeout(function(){ timer = null; flush(); }, FLUSH_INTERVAL_MS);
  }

  // Off-hot-path serialisation. The rrweb emit fires synchronously
  // inline with DOM mutations; if we JSON.stringify there, the main
  // thread backs up on busy pages. Push the raw event into a queue,
  // schedule a microtask once per batch to drain into the NDJSON buf.
  var rawQueue = [];
  function drainRawQueue(){
    pendingSerialise = null;
    for (var i = 0; i < rawQueue.length; i++) {
      var event = rawQueue[i];
      var line;
      try {
        line = JSON.stringify({
          tabPageId: tabPageId,
          ts: (event && typeof event.timestamp === 'number') ? event.timestamp : Date.now(),
          type: event && event.type,
          data: event && event.data
        });
      } catch (err) { continue; }
      if (buf.length >= BUFFER_CAP) {
        // Drop oldest to make room. Holding all events in memory
        // forever on a runaway page would OOM the tab; lossy
        // replay is better than a crashed page.
        buf.shift();
        dropped++;
      }
      buf.push(line);
    }
    rawQueue = [];
    armFlushTimer();
  }

  window.rrweb.record({
    maskInputOptions: { password: true },
    // Aggressive sampling so heavy SPA pages do not generate the
    // ~1k events/sec that locked the browser in early dogfood.
    sampling: {
      mousemove: false,
      scroll: 250,
      media: 500,
      input: 'last'
    },
    recordCanvas: false,
    emit: function(event){
      rawQueue.push(event);
      if (pendingSerialise === null) {
        pendingSerialise = (typeof queueMicrotask === 'function')
          ? (queueMicrotask(drainRawQueue), 1)
          : setTimeout(drainRawQueue, 0);
      }
    }
  });

  function flushNow(){
    // Drain any pending serialisation, then send synchronously.
    if (rawQueue.length > 0) drainRawQueue();
    flush();
  }
  window.addEventListener('pagehide', flushNow);
  window.addEventListener('visibilitychange', function(){
    if (document.visibilityState === 'hidden') flushNow();
  });
})();
})();`
}

/** Test-only accessor so we can assert the vendor file is loadable. */
export function _rrwebVendorSizeForTesting(): number {
  return RRWEB_UMD_SOURCE.length
}
