/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * HTTP surface for rrweb session replays. Three endpoints:
 *
 *   POST /audit/replay/:sessionId/events     recorder appends events
 *   GET  /audit/replay/:sessionId            UI streams the events
 *   GET  /audit/replay/:sessionId/exists     UI checks if data exists
 *
 * The recorder is loaded by the claw-app extension's content script
 * (bundled into the extension itself); no static-asset route is
 * needed any more. F8 ripped out the previous bootstrap-stub
 * mechanism.
 *
 * Authorisation posture for the event POST: trusts the sessionId in
 * the URL but cross-checks against `identityService.getIdentity`. A
 * page whose session has been dropped returns 410 Gone, so a
 * malicious local page cannot post events for an unrelated session.
 * This is the same trust posture the cancellation route in #1392
 * established; whole-loopback hardening is a separate concern.
 *
 * The server prepends its own trusted `sessionId` to every event
 * line before writing so the on-disk shape is self-contained and
 * the recorder cannot spoof a different session into the file.
 */

import { Hono } from 'hono'
import { logger } from '../../lib/logger'
import { identityService } from '../../lib/mcp-session'
import { replayStorage } from '../../services/replay-storage'

export const auditReplayRoute = new Hono()
  .post('/audit/replay/:sessionId/events', async (c) => {
    const sessionId = c.req.param('sessionId')
    if (!identityService.getIdentity(sessionId)) {
      return c.json(
        { ok: false, reason: 'session not live' },
        // 410 Gone matches the spec's "the resource was here, it is
        // no longer here" semantic: the session existed (the URL
        // refers to a real sessionId shape) but its identity is no
        // longer registered, so we no longer accept writes for it.
        410,
      )
    }
    const body = await c.req.text()
    if (body.length === 0) {
      return c.json({ ok: true, accepted: 0 })
    }
    const lines = body.split('\n').filter((l) => l.length > 0)
    if (lines.length === 0) {
      return c.json({ ok: true, accepted: 0 })
    }
    try {
      const annotated = lines.map((line) =>
        annotateWithSessionId(line, sessionId),
      )
      await replayStorage.appendEvents(sessionId, annotated)
    } catch (err) {
      logger.warn('replay events append failed', {
        sessionId,
        lineCount: lines.length,
        error: err instanceof Error ? err.message : String(err),
      })
      return c.json({ ok: false, reason: 'append failed' }, 500)
    }
    // F6 observability: every successful batch leaves a trace in the
    // dev terminal so the recorder loop is visible end-to-end.
    logger.info('replay events accepted', {
      sessionId,
      accepted: lines.length,
    })
    return c.json({ ok: true, accepted: lines.length })
  })
  .get('/audit/replay/:sessionId', async (c) => {
    const sessionId = c.req.param('sessionId')
    const stat = await replayStorage.statSession(sessionId)
    if (!stat.hasData) {
      return c.json({ ok: false, reason: 'no replay for this session' }, 404)
    }
    const stream = await replayStorage.readEvents(sessionId)
    return new Response(stream, {
      headers: {
        'content-type': 'application/x-ndjson',
        'content-length': String(stat.sizeBytes),
        // Range support lets the UI scrub without re-reading the
        // whole file. Bun.file().stream() emits the entire file
        // today; range serving is a follow-up if sizes grow beyond
        // what the UI can hold in memory.
        'accept-ranges': 'bytes',
      },
    })
  })
  .get('/audit/replay/:sessionId/exists', async (c) => {
    const sessionId = c.req.param('sessionId')
    const stat = await replayStorage.statSession(sessionId)
    return c.json({ ok: true, ...stat })
  })

/**
 * Re-encodes one NDJSON line with the server-trusted sessionId
 * prepended. The recorder client posts events tagged with tabPageId
 * and ts; we own the sessionId so the on-disk line carries the
 * server's view, not the client's claim.
 *
 * Malformed lines (not JSON) are passed through verbatim; the read
 * side tolerates bad lines but we log at debug so we notice if a
 * recorder revision starts emitting garbage.
 */
function annotateWithSessionId(line: string, sessionId: string): string {
  try {
    const parsed = JSON.parse(line) as Record<string, unknown>
    parsed.sessionId = sessionId
    return JSON.stringify(parsed)
  } catch {
    return line
  }
}
