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

  it('output contains the inlined rrweb UMD source', () => {
    const script = buildInitScript({
      sessionId: 'session-abc',
      tabPageId: 42,
      cockpitOrigin: 'http://127.0.0.1:9200',
    })
    // The rrweb UMD bundle starts with the universal-module-definition
    // factory; the safest signature to grep for is the literal name.
    expect(script).toContain('rrweb')
    expect(script).toContain('window.rrweb.record')
  })

  it('bakes sessionId + tabPageId + cockpitOrigin into the output', () => {
    const script = buildInitScript({
      sessionId: 'sess-1234',
      tabPageId: 99,
      cockpitOrigin: 'http://127.0.0.1:9200',
    })
    expect(script).toContain('"sess-1234"')
    // The URL is built at runtime via string concatenation, so the
    // literals appear separately in the output.
    expect(script).toContain('"http://127.0.0.1:9200"')
    expect(script).toContain("'/audit/replay/'")
    // Per-session variable assignments include the tab id verbatim.
    expect(script).toContain('var tabPageId = 99')
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
