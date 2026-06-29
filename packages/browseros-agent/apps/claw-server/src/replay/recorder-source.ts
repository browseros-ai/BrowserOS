/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Builds the per-session JavaScript blob the cockpit injects into
 * every agent-driven BrowserOS page via CDP
 * `Page.addScriptToEvaluateOnNewDocument`.
 *
 * F5 design (bootstrap stub + async UMD fetch):
 *
 * The injected script is a small (~2KB) stub. It does NOT contain
 * the rrweb UMD source. Instead, it appends a `<script async src>`
 * tag pointing at the cockpit's static UMD route. The browser
 * downloads and parses rrweb off the document-load critical path;
 * once the script loads, the stub's `onload` handler starts the
 * recorder with the throttled config.
 *
 * Earlier attempts inlined the 260KB UMD into the injected payload.
 * `addScriptToEvaluateOnNewDocument` runs that script BEFORE any
 * page script on every new document, and the synchronous parse of
 * 260KB stalled the renderer enough to drop the CDP connection
 * (S2 in the recorder-stability plan). The bootstrap stub avoids
 * the stall because the script body adds the async tag and returns
 * immediately.
 *
 * Trade-off accepted: pages with a strict `script-src` CSP that
 * does not whitelist the cockpit's loopback origin will refuse the
 * async script load. The stub's `onerror` logs to the page console
 * so the operator can diagnose. For sites where this matters we
 * can revisit a Worker- or sandboxed-frame-based variant.
 *
 * Throttling decisions (used when the recorder boots):
 *
 *   - `sampling.mousemove: false` drops MouseMove entirely.
 *   - `sampling.scroll: 250` rate-limits scroll to one event per
 *     250ms.
 *   - `sampling.media: 500` rate-limits media + canvas updates.
 *   - `sampling.input: 'last'` only records the final value of
 *     each input field rather than every keystroke.
 *   - `recordCanvas: false` skips canvas capture entirely.
 *   - The emit handler defers JSON.stringify into a microtask so
 *     the rrweb hot path returns to the page as fast as possible.
 *   - A 500-event in-memory cap drops the oldest events with a
 *     console warn if the page produces faster than we can flush.
 *
 * `window.__browserosClawReplayInstalled` guards against double-
 * install when `addScriptToEvaluateOnNewDocument` fires twice for
 * the same document.
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'

/**
 * The vendored UMD bytes are read here only so the size accessor
 * stays accurate for tests; the buildInitScript output no longer
 * inlines the bytes. The audit-replay route is the one that
 * actually serves the file to the recorder stub.
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
var sessionId = ${JSON.stringify(sessionId)};
var tabPageId = ${JSON.stringify(tabPageId)};
var cockpitOrigin = ${JSON.stringify(cockpitOrigin)};
var eventsUrl = cockpitOrigin + '/audit/replay/' + sessionId + '/events';
var rrwebUrl = cockpitOrigin + '/audit/replay/static/rrweb.umd.min.js';

function bootRecorder(){
  if (!window.rrweb || typeof window.rrweb.record !== 'function') {
    try { console.warn('[browseros-claw replay] rrweb unavailable after load'); } catch(e){}
    return;
  }

  var buf = [];
  var BUFFER_CAP = 500;
  var FLUSH_INTERVAL_MS = 2500;
  var timer = null;
  var dropped = 0;
  var pendingSerialise = null;
  var rawQueue = [];

  function send(body){
    try {
      if (typeof navigator.sendBeacon === 'function' && document.visibilityState === 'hidden') {
        navigator.sendBeacon(eventsUrl, new Blob([body], { type: 'application/x-ndjson' }));
        return;
      }
      fetch(eventsUrl, {
        method: 'POST',
        headers: { 'content-type': 'application/x-ndjson' },
        body: body,
        credentials: 'omit'
      }).catch(function(err){
        try { console.warn('[browseros-claw replay] events fetch failed', err && err.message); } catch(e){}
      });
    } catch (e) {
      try { console.warn('[browseros-claw replay] send threw', e && e.message); } catch(e2){}
    }
  }

  function flush(){
    if (buf.length === 0) return;
    var body = buf.join('\\n');
    buf = [];
    if (dropped > 0) {
      try { console.warn('[browseros-claw replay] dropped', dropped, 'events under buffer pressure'); } catch(e){}
      dropped = 0;
    }
    send(body);
  }

  function armFlushTimer(){
    if (timer !== null) return;
    timer = setTimeout(function(){ timer = null; flush(); }, FLUSH_INTERVAL_MS);
  }

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
      if (buf.length >= BUFFER_CAP) { buf.shift(); dropped++; }
      buf.push(line);
    }
    rawQueue = [];
    armFlushTimer();
  }

  try {
    window.rrweb.record({
      maskInputOptions: { password: true },
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
    try { console.info('[browseros-claw replay] recorder online', { sessionId: sessionId, tabPageId: tabPageId }); } catch(e){}
  } catch (err) {
    try { console.warn('[browseros-claw replay] rrweb.record threw', err && err.message); } catch(e){}
    return;
  }

  function flushNow(){
    if (rawQueue.length > 0) drainRawQueue();
    flush();
  }
  window.addEventListener('pagehide', flushNow);
  window.addEventListener('visibilitychange', function(){
    if (document.visibilityState === 'hidden') flushNow();
  });
}

// Append the rrweb UMD as an async script. document.documentElement
// is reliably available even at this very early script-slot point
// (the documentElement exists before head/body when running in the
// addScriptToEvaluateOnNewDocument slot); falling back to body /
// documentElement covers the case where the slot runs even earlier.
function injectScriptTag(){
  var s = document.createElement('script');
  s.async = true;
  s.src = rrwebUrl;
  s.crossOrigin = 'anonymous';
  s.onload = bootRecorder;
  s.onerror = function(){
    try { console.warn('[browseros-claw replay] umd load failed', rrwebUrl); } catch(e){}
  };
  var root = document.documentElement || document.head || document.body;
  if (root) {
    root.appendChild(s);
  } else {
    // Extremely early; wait for the document tree to materialise.
    document.addEventListener('readystatechange', function once(){
      if (document.documentElement) {
        document.removeEventListener('readystatechange', once);
        (document.documentElement || document.head || document.body).appendChild(s);
      }
    });
  }
}

injectScriptTag();
})();`
}

/** Test-only accessor so we can assert the vendor file is loadable. */
export function _rrwebVendorSizeForTesting(): number {
  return RRWEB_UMD_SOURCE.length
}
