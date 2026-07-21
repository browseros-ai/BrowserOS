import { afterEach, describe, expect, it, mock } from 'bun:test'

const { fetchReplayEvents, fetchReplayMetadata, replayEventsRevision } =
  await import('./replay.hooks')

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('replay queries', () => {
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
    const metadata = {
      hasData: true,
      complete: true,
      firstEventAt: 1_000,
      lastEventAt: 4_000,
      sizeBytes: 512,
      tabs: [],
    }
    const request = mock(async () => Response.json(metadata))
    globalThis.fetch = request as unknown as typeof fetch

    await expect(
      fetchReplayMetadata({ sessionId: 'session/with slash' }),
    ).resolves.toEqual(metadata)
    expect(request).toHaveBeenCalledWith(
      'http://127.0.0.1:9200/api/v1/sessions/session%2Fwith%20slash/recording',
      expect.objectContaining({ method: 'GET' }),
    )
  })

  it('preserves the empty metadata shape when no replay exists', async () => {
    const metadata = { hasData: false, complete: true, sizeBytes: 0, tabs: [] }
    globalThis.fetch = mock(async () =>
      Response.json(metadata),
    ) as unknown as typeof fetch

    await expect(
      fetchReplayMetadata({ sessionId: 'session-1' }),
    ).resolves.toEqual(metadata)
  })

  it('parses valid replay lines and skips malformed lines', async () => {
    const body = [
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
    const request = mock(async () => new Response(body))
    globalThis.fetch = request as unknown as typeof fetch

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
    expect(request).toHaveBeenCalledWith(
      'http://127.0.0.1:9200/api/v1/sessions/session-1/recording/events',
    )
  })
})
