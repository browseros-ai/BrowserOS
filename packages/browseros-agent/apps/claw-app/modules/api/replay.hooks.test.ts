import { afterAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { RecordingMetadata } from '@browseros/claw-api'
import { ApiResponseError } from '@browseros/claw-api-client'
import * as _client from './client'

let metadataResponse: RecordingMetadata = {
  hasData: false,
  complete: true,
  sizeBytes: 0,
  tabs: [],
}
let eventsResponse = ''
const getRecording = mock(async () => metadataResponse)
const downloadRecordingEvents = mock(async () => eventsResponse)
const actualApiClient = _client.apiClient

mock.module('./client', () => ({
  ..._client,
  apiClient: async () =>
    Object.assign(await actualApiClient(), {
      getRecording,
      downloadRecordingEvents,
    }),
}))

const {
  fetchReplayEvents,
  fetchReplayMetadata,
  replayEventsRevision,
  replayMetadataRefetchInterval,
  REPLAY_METADATA_POLL_MS,
  useReplayEvents,
} = await import('./replay.hooks')

afterAll(() => mock.restore())

beforeEach(() => {
  getRecording.mockClear()
  downloadRecordingEvents.mockClear()
})

describe('replay queries', () => {
  it('isolates event snapshots by metadata revision', () => {
    expect(
      Array.from(
        useReplayEvents.getKey({
          sessionId: 'session-1',
          revision: 'revision-2',
        }),
      ),
    ).toEqual([
      'replay',
      'events',
      { sessionId: 'session-1', revision: 'revision-2' },
    ])
  })

  it('changes the event revision when late recording metadata advances', () => {
    const metadata = {
      hasData: true,
      complete: true,
      lastEventAt: 2_000,
      sizeBytes: 128,
      tabs: [
        {
          tabId: 9,
          complete: true,
          firstEventAt: 1_000,
          lastEventAt: 2_000,
          segments: [
            {
              documentId: 'document-a',
              firstEventAt: 1_000,
              lastEventAt: 2_000,
              sizeBytes: 128,
              eventCount: 2,
              hasGap: false,
            },
          ],
        },
      ],
    }
    const first = replayEventsRevision(metadata)
    expect(replayEventsRevision({ ...metadata })).toBe(first)
    expect(
      replayEventsRevision({
        ...metadata,
        lastEventAt: 3_000,
        sizeBytes: 192,
      }),
    ).not.toBe(first)
  })

  it('fetches canonical recording metadata', async () => {
    metadataResponse = {
      hasData: true,
      complete: true,
      firstEventAt: 1_000,
      lastEventAt: 4_000,
      sizeBytes: 512,
      tabs: [],
    }

    await expect(
      fetchReplayMetadata({ sessionId: 'session/with slash' }),
    ).resolves.toBe(metadataResponse)
    expect(getRecording).toHaveBeenCalledWith({
      sessionId: 'session/with slash',
    })
  })

  it('preserves the empty metadata shape when no replay exists', async () => {
    metadataResponse = {
      hasData: false,
      complete: true,
      sizeBytes: 0,
      tabs: [],
    }

    await expect(fetchReplayMetadata({ sessionId: 'session-1' })).resolves.toBe(
      metadataResponse,
    )
  })

  it('parses valid replay lines and skips malformed lines', async () => {
    eventsResponse = [
      JSON.stringify({
        sessionId: 'session-1',
        documentId: 'document-b',
        targetId: 'target-b',
        tabId: 9,
        ts: 2_000,
        type: 4,
        data: { width: 1280, height: 720 },
      }),
      '{not-json',
      JSON.stringify({
        sessionId: 'session-1',
        documentId: 'document-invalid',
        tabId: 9,
        ts: 2_500,
        type: 3,
        data: {},
      }),
      JSON.stringify({
        sessionId: 'session-1',
        documentId: 'document-a',
        targetId: 'target-a',
        tabId: 3,
        ts: 3_000,
        type: 2,
        data: {},
      }),
    ].join('\n')

    await expect(
      fetchReplayEvents({ sessionId: 'session-1' }),
    ).resolves.toEqual({
      events: [
        {
          sessionId: 'session-1',
          documentId: 'document-b',
          targetId: 'target-b',
          tabId: 9,
          ts: 2_000,
          type: 4,
          data: { width: 1280, height: 720 },
        },
        {
          sessionId: 'session-1',
          documentId: 'document-a',
          targetId: 'target-a',
          tabId: 3,
          ts: 3_000,
          type: 2,
          data: {},
        },
      ],
      tabIds: [9, 3],
      documentIds: ['document-b', 'document-a'],
    })
    expect(downloadRecordingEvents).toHaveBeenCalledWith({
      sessionId: 'session-1',
    })
  })

  it('maps a missing recording event stream to an empty bundle', async () => {
    downloadRecordingEvents.mockImplementationOnce(async () => {
      throw new ApiResponseError(
        Response.json(
          { code: 'recording_not_found', message: 'recording not found' },
          { status: 404 },
        ),
      )
    })

    await expect(fetchReplayEvents({ sessionId: 'missing' })).resolves.toEqual({
      events: [],
      tabIds: [],
      documentIds: [],
    })
  })
})

describe('replayMetadataRefetchInterval', () => {
  it('polls while the session is live', () => {
    expect(
      replayMetadataRefetchInterval({
        status: 'live',
        endedAt: null,
        hasData: false,
      }),
    ).toBe(REPLAY_METADATA_POLL_MS)
  })

  it('stops once the recording has appeared', () => {
    expect(
      replayMetadataRefetchInterval({
        status: 'done',
        endedAt: 1_000,
        hasData: true,
        now: 1_500,
      }),
    ).toBe(false)
  })

  it('keeps polling briefly after the session ends until the recording appears', () => {
    // Ended 5s ago, still no recording: a trailing batch may still land.
    expect(
      replayMetadataRefetchInterval({
        status: 'done',
        endedAt: 1_000,
        hasData: false,
        now: 6_000,
      }),
    ).toBe(REPLAY_METADATA_POLL_MS)
  })

  it('stops after the grace window even if the recording never arrived', () => {
    // Ended 31s ago with no recording: give up rather than poll forever.
    expect(
      replayMetadataRefetchInterval({
        status: 'done',
        endedAt: 1_000,
        hasData: false,
        now: 32_000,
      }),
    ).toBe(false)
  })

  it('does not poll a terminal session with no end timestamp', () => {
    expect(
      replayMetadataRefetchInterval({
        status: 'done',
        endedAt: null,
        hasData: false,
      }),
    ).toBe(false)
  })
})
