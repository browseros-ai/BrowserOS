import type { AppendRecordingEventsResponse } from '@browseros/claw-api'
import type { Handler } from 'hono'
import { getTabTargetMap } from '../../../lib/tab-targets'
import {
  type RecordingEventInput,
  recordingStore,
} from '../../../services/recordings'
import { apiError } from '../errors'
import type { RequestContextEnv } from '../request-context'
import { positiveInteger } from '../validation'

export interface RecordingIdentity {
  tabId: number
  documentId: string
}

export interface RecordingHandlerDependencies {
  appendRecordingEvents(
    identity: RecordingIdentity,
    events: RecordingEventInput[],
    batchId: string,
    hasGap: boolean,
  ): Promise<AppendRecordingEventsResponse>
}

export interface RecordingHandlers {
  ingest: Handler<RequestContextEnv>
}

export function createRecordingHandlers(
  dependencies: RecordingHandlerDependencies,
): RecordingHandlers {
  return {
    ingest: async (c) => {
      const contentType = c.req.header('content-type') ?? ''
      if (!contentType.toLowerCase().startsWith('application/x-ndjson')) {
        return apiError(
          c,
          400,
          'invalid_request',
          'content-type must be application/x-ndjson',
        )
      }
      const tabId = positiveInteger(c.req.header('x-recording-tab-id') ?? '')
      const documentId = c.req.header('x-recording-document-id') ?? ''
      const batchId = c.req.header('x-recording-batch-id') ?? ''
      const hasGap = recordingGap(c.req.header('x-recording-has-gap'))
      if (
        tabId === null ||
        !isChromeDocumentId(documentId) ||
        batchId.length === 0 ||
        hasGap === null
      ) {
        return apiError(
          c,
          400,
          'invalid_request',
          'recording tab, document, batch, and gap headers are invalid',
        )
      }
      const parsed = parseRecordingEvents(await c.req.text())
      if (parsed.events.length === 0) return c.json({ accepted: 0 })
      return c.json(
        await dependencies.appendRecordingEvents(
          { tabId, documentId },
          parsed.events,
          batchId,
          hasGap || parsed.droppedLines > 0,
        ),
      )
    },
  }
}

export const productionRecordingHandlers = createRecordingHandlers({
  async appendRecordingEvents(identity, events, batchId, hasGap) {
    const targetId =
      (await getTabTargetMap()?.targetForTab(identity.tabId)) ?? null
    const appended = await recordingStore.appendBatch({
      documentId: identity.documentId,
      tabId: identity.tabId,
      targetId,
      events,
      batchId,
      hasGap,
    })
    return { accepted: appended ? events.length : 0 }
  },
})

function recordingGap(raw: string | undefined): boolean | null {
  if (raw === undefined || raw === 'false') return false
  if (raw === 'true') return true
  return null
}

function isChromeDocumentId(value: string): boolean {
  return /^[0-9a-f]{32}$/i.test(value)
}

/** Recorder NDJSON is tolerant: malformed lines and events without a finite timestamp are dropped. */
function parseRecordingEvents(ndjson: string): {
  events: RecordingEventInput[]
  droppedLines: number
} {
  const events: RecordingEventInput[] = []
  let droppedLines = 0
  for (const line of ndjson.split('\n')) {
    if (!line.trim()) continue
    try {
      const event = JSON.parse(line) as Record<string, unknown>
      if (typeof event.ts !== 'number' || !Number.isFinite(event.ts)) {
        droppedLines++
        continue
      }
      events.push({ ts: event.ts, type: event.type, data: event.data })
    } catch {
      droppedLines++
    }
  }
  return { events, droppedLines }
}
