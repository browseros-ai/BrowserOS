/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  _rrwebVendorSizeForTesting,
  buildInitScript,
} from '../../src/replay/recorder-source'

describe('recorder-source', () => {
  it('vendor UMD is loaded at module init', () => {
    // rrweb 2.0.1 minified is ~260 KB. A safety margin in either
    // direction catches both a missing vendor file (size 0) and a
    // vendor file that has accidentally grown enormous.
    expect(_rrwebVendorSizeForTesting()).toBeGreaterThan(100_000)
    expect(_rrwebVendorSizeForTesting()).toBeLessThan(1_000_000)
  })

  it('output is a small bootstrap stub, NOT the full rrweb UMD', () => {
    const script = buildInitScript({
      sessionId: 'session-abc',
      tabPageId: 42,
      cockpitOrigin: 'http://127.0.0.1:9200',
    })
    // The stub references rrweb but does NOT inline the 260KB UMD.
    expect(script).toContain('window.rrweb.record')
    expect(script.length).toBeLessThan(8_000)
  })

  it('bakes sessionId + tabPageId + cockpitOrigin into the output', () => {
    const script = buildInitScript({
      sessionId: 'sess-1234',
      tabPageId: 99,
      cockpitOrigin: 'http://127.0.0.1:9200',
    })
    expect(script).toContain('"sess-1234"')
    expect(script).toContain('"http://127.0.0.1:9200"')
    expect(script).toContain('var tabPageId = 99')
  })

  it('injects the static UMD URL via an async script tag', () => {
    const script = buildInitScript({
      sessionId: 's',
      tabPageId: 1,
      cockpitOrigin: 'http://127.0.0.1:9200',
    })
    // The stub builds `cockpitOrigin + '/audit/replay/static/rrweb.umd.min.js'`
    // and creates a script tag with that src.
    expect(script).toContain('/audit/replay/static/rrweb.umd.min.js')
    expect(script).toContain('createElement')
    expect(script).toContain('s.async = true')
    expect(script).toContain('s.onload')
    expect(script).toContain('s.onerror')
  })

  it('escapes a sessionId that contains quotes', () => {
    const script = buildInitScript({
      sessionId: 'sess "with quotes',
      tabPageId: 1,
      cockpitOrigin: 'http://x',
    })
    // The literal string must appear escaped, never raw, so the IIFE
    // parses cleanly and a malformed sessionId cannot break out of
    // the string context.
    expect(script).not.toContain('sess "with quotes')
    expect(script).toContain(JSON.stringify('sess "with quotes'))
  })

  it('guards against double-install with __browserosClawReplayInstalled', () => {
    const script = buildInitScript({
      sessionId: 's',
      tabPageId: 1,
      cockpitOrigin: 'http://x',
    })
    expect(script).toContain('__browserosClawReplayInstalled')
  })

  it('is deterministic across calls with the same inputs', () => {
    const a = buildInitScript({
      sessionId: 'same',
      tabPageId: 1,
      cockpitOrigin: 'http://x',
    })
    const b = buildInitScript({
      sessionId: 'same',
      tabPageId: 1,
      cockpitOrigin: 'http://x',
    })
    expect(a).toBe(b)
  })
})
