import { describe, expect, it } from 'bun:test'
import type {
  NewRecordingBatch,
  RecordingOutbox,
  StoredRecordingBatch,
} from './recordings-outbox'
import {
  createRecordingsRelay,
  RECORDINGS_QUEUE_MAX_BYTES,
} from './recordings-relay'

const serverBaseUrl = 'http://127.0.0.1:9511'
const documentIds = {
  retrying: '33D25F3CF060E81B14070BC356FF1871',
  restart: '8395FF2EF4A1D8579F1917B3B54ADECE',
  oldSidecar: '9E84CDCAB8762569B5B109D125F60147',
  gap: 'A18F47A71C2B7DEF81230123456789AC',
  evicted: 'B18F47A71C2B7DEF81230123456789AD',
  retained: 'C18F47A71C2B7DEF81230123456789AE',
  oversized: 'D18F47A71C2B7DEF81230123456789AF',
} as const

function createMemoryOutbox(): RecordingOutbox & {
  batches: StoredRecordingBatch[]
  gaps: Map<string, string>
} {
  let sequence = 0
  let gapSequence = 0
  const batches: StoredRecordingBatch[] = []
  const gaps = new Map<string, string>()
  return {
    batches,
    gaps,
    async add(batch: NewRecordingBatch) {
      batches.push({ ...batch, sequence: ++sequence })
    },
    async list() {
      return [...batches].sort((left, right) => left.sequence - right.sequence)
    },
    async remove(batchId) {
      const index = batches.findIndex((batch) => batch.batchId === batchId)
      if (index !== -1) batches.splice(index, 1)
    },
    async markGap(documentId, tabId) {
      gaps.set(documentId, `${tabId.toString()}-${(++gapSequence).toString()}`)
    },
    async getGap(documentId) {
      const token = gaps.get(documentId)
      return token ? { documentId, tabId: 0, token } : undefined
    },
    async clearGap(documentId, token) {
      if (gaps.get(documentId) === token) gaps.delete(documentId)
    },
  }
}

interface FakeTimer {
  id: number
  at: number
  callback: () => void | Promise<void>
}

function createFakeClock() {
  let now = 0
  let nextTimerId = 1
  const timers: FakeTimer[] = []
  return {
    now: () => now,
    setTimeout(callback: () => void, delayMs: number) {
      const timer = { id: nextTimerId++, at: now + delayMs, callback }
      timers.push(timer)
      return timer.id as unknown as ReturnType<typeof globalThis.setTimeout>
    },
    clearTimeout(handle: ReturnType<typeof globalThis.setTimeout>) {
      const index = timers.findIndex((timer) => timer.id === Number(handle))
      if (index !== -1) timers.splice(index, 1)
    },
    async advanceBy(delayMs: number) {
      now += delayMs
      timers.sort((left, right) => left.at - right.at)
      const due = timers.splice(
        0,
        timers.findLastIndex((timer) => timer.at <= now) + 1,
      )
      for (const timer of due) await timer.callback()
    },
    pendingTimers: () => timers.length,
  }
}

function requestHeader(init: RequestInit | undefined, name: string): string {
  return new Headers(init?.headers).get(name) ?? ''
}

function systemResponse(version: number | null = 2, maxBytes = 4_194_304) {
  return Response.json({
    capabilities: {
      recordingIngestVersion: version ?? undefined,
      recordingIngestMaxBytes: maxBytes,
    },
  })
}

describe('createRecordingsRelay', () => {
  it('persists before delivery and retries stable document batches without tab discovery', async () => {
    const clock = createFakeClock()
    const outbox = createMemoryOutbox()
    const attempts: Array<{
      url: string
      body: string
      batchId: string
      tabId: string
      documentId: string
      sessionId: string
      pageId: string
      targetId: string
    }> = []
    let serverUp = false
    const relay = createRecordingsRelay({
      resolveServerBaseUrl: async () => serverBaseUrl,
      outbox,
      fetch: async (input, init) => {
        const url = String(input)
        if (url.endsWith('/api/v1/system')) return systemResponse()
        attempts.push({
          url,
          body: String(init?.body),
          batchId: requestHeader(init, 'X-Recording-Batch-Id'),
          tabId: requestHeader(init, 'X-Recording-Tab-Id'),
          documentId: requestHeader(init, 'X-Recording-Document-Id'),
          sessionId: requestHeader(init, 'X-Browser-Session-Id'),
          pageId: requestHeader(init, 'X-Recording-Page-Id'),
          targetId: requestHeader(init, 'X-Recording-Target-Id'),
        })
        if (!serverUp) throw new TypeError('connection refused')
        return Response.json({ accepted: 1 })
      },
      now: clock.now,
      setTimeout: clock.setTimeout,
      clearTimeout: clock.clearTimeout,
      warn: () => {},
    })

    await relay.post(42, documentIds.retrying, 'first')
    await relay.post(42, documentIds.retrying, 'second')

    expect(outbox.batches.map((batch) => batch.ndjson)).toEqual([
      'first',
      'second',
    ])
    expect(attempts.some(({ url }) => url.endsWith('/api/v1/tabs'))).toBe(false)
    const firstBatchId = outbox.batches[0]?.batchId
    expect(firstBatchId).toBeString()

    serverUp = true
    await clock.advanceBy(5_000)

    const successful = attempts.slice(-2)
    expect(successful.map(({ body }) => body)).toEqual(['first', 'second'])
    expect(successful[0]?.batchId).toBe(firstBatchId)
    expect(successful[0]).toMatchObject({
      url: `${serverBaseUrl}/api/v1/recordings/events`,
      tabId: '42',
      documentId: documentIds.retrying,
      sessionId: '',
      pageId: '',
      targetId: '',
    })
    expect(outbox.batches).toEqual([])
    expect(clock.pendingTimers()).toBe(0)
  })

  it('resumes the same durable batch after a background restart', async () => {
    const outbox = createMemoryOutbox()
    let firstAttemptId = ''
    const firstRelay = createRecordingsRelay({
      resolveServerBaseUrl: async () => serverBaseUrl,
      outbox,
      fetch: async (input, init) => {
        if (String(input).endsWith('/api/v1/system')) return systemResponse()
        firstAttemptId = requestHeader(init, 'X-Recording-Batch-Id')
        throw new TypeError('worker stopped')
      },
      setTimeout: () =>
        1 as unknown as ReturnType<typeof globalThis.setTimeout>,
      warn: () => {},
    })
    await firstRelay.post(7, documentIds.restart, 'persisted')

    let resumedId = ''
    const resumedRelay = createRecordingsRelay({
      resolveServerBaseUrl: async () => serverBaseUrl,
      outbox,
      fetch: async (input, init) => {
        if (String(input).endsWith('/api/v1/system')) return systemResponse()
        resumedId = requestHeader(init, 'X-Recording-Batch-Id')
        return Response.json({ accepted: 1 })
      },
      warn: () => {},
    })
    await resumedRelay.start()

    expect(resumedId).toBe(firstAttemptId)
    expect(outbox.batches).toEqual([])
  })

  it('keeps events until an older sidecar advertises ingest v2', async () => {
    const clock = createFakeClock()
    const outbox = createMemoryOutbox()
    let version: number | null = null
    const postedBodies: string[] = []
    const relay = createRecordingsRelay({
      resolveServerBaseUrl: async () => serverBaseUrl,
      outbox,
      fetch: async (input, init) => {
        if (String(input).endsWith('/api/v1/system')) {
          return systemResponse(version)
        }
        postedBodies.push(String(init?.body))
        return Response.json({ accepted: 1 })
      },
      now: clock.now,
      setTimeout: clock.setTimeout,
      clearTimeout: clock.clearTimeout,
      warn: () => {},
    })

    await relay.post(1, documentIds.oldSidecar, 'queued')
    expect(postedBodies).toEqual([])
    expect(outbox.batches).toHaveLength(1)

    version = 2
    await clock.advanceBy(60_000)

    expect(postedBodies).toEqual(['queued'])
    expect(outbox.batches).toEqual([])
  })

  it('sends durable gap metadata and requests a new checkpoint after ack', async () => {
    const outbox = createMemoryOutbox()
    const recoveredTabs: number[] = []
    let gapHeader = ''
    const relay = createRecordingsRelay({
      resolveServerBaseUrl: async () => serverBaseUrl,
      outbox,
      fetch: async (input, init) => {
        if (String(input).endsWith('/api/v1/system')) return systemResponse()
        gapHeader = requestHeader(init, 'X-Recording-Has-Gap')
        return Response.json({ accepted: 1 })
      },
      warn: () => {},
    })
    relay.onTabRecoveredAfterLoss((tabId) => recoveredTabs.push(tabId))

    await relay.post(12, documentIds.gap, 'checkpoint', true)

    expect(gapHeader).toBe('true')
    expect(outbox.gaps.has(documentIds.gap)).toBe(false)
    expect(recoveredTabs).toEqual([12])
  })

  it('bounds the durable outbox and marks the evicted document incomplete', async () => {
    const outbox = createMemoryOutbox()
    const relay = createRecordingsRelay({
      resolveServerBaseUrl: async () => serverBaseUrl,
      outbox,
      fetch: async (input) => {
        if (String(input).endsWith('/api/v1/system')) {
          return systemResponse(null)
        }
        return Response.json({ accepted: 1 })
      },
      setTimeout: () =>
        1 as unknown as ReturnType<typeof globalThis.setTimeout>,
      warn: () => {},
    })

    const batchBytes = Math.floor(RECORDINGS_QUEUE_MAX_BYTES * 0.6)
    await relay.post(1, documentIds.evicted, 'a'.repeat(batchBytes))
    await relay.post(2, documentIds.retained, 'b'.repeat(batchBytes))

    expect(
      outbox.batches.reduce((sum, batch) => sum + batch.bytes, 0),
    ).toBeLessThanOrEqual(RECORDINGS_QUEUE_MAX_BYTES)
    expect(outbox.gaps.has(documentIds.evicted)).toBe(true)
    expect(
      outbox.batches.every((batch) => batch.documentId !== documentIds.evicted),
    ).toBe(true)
  })

  it('drops a server-rejected oversized event but marks the next batch gapped', async () => {
    const outbox = createMemoryOutbox()
    const gapHeaders: string[] = []
    const relay = createRecordingsRelay({
      resolveServerBaseUrl: async () => serverBaseUrl,
      outbox,
      fetch: async (input, init) => {
        if (String(input).endsWith('/api/v1/system'))
          return systemResponse(2, 100)
        gapHeaders.push(requestHeader(init, 'X-Recording-Has-Gap'))
        return Response.json({ accepted: 1 })
      },
      warn: () => {},
    })

    await relay.post(5, documentIds.oversized, 'x'.repeat(101))
    expect(outbox.gaps.has(documentIds.oversized)).toBe(true)
    await relay.post(5, documentIds.oversized, 'small')

    expect(gapHeaders).toEqual(['true'])
    expect(outbox.gaps.has(documentIds.oversized)).toBe(false)
  })
})
