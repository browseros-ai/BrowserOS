import { describe, expect, mock, test } from 'bun:test'
import { RECORDING_INGEST_MAX_BYTES } from '@browseros/shared/constants/limits'
import { createRecordingHandlers } from '../../../src/api/http/handlers/recordings'
import { httpTestApp } from './fixtures'

const validHeaders = {
  'content-type': 'application/x-ndjson',
  'x-recording-tab-id': '101',
  'x-recording-document-id': '33D25F3CF060E81B14070BC356FF1871',
  'x-recording-batch-id': 'batch-1',
}

describe('recording HTTP handlers', () => {
  test('parses document batches and marks dropped lines as a gap', async () => {
    const appendRecordingEvents = mock(async () => ({ accepted: 2 }))
    const app = httpTestApp({
      recordings: createRecordingHandlers({ appendRecordingEvents }),
    })
    const response = await app.request('/api/v1/recordings/events', {
      method: 'POST',
      headers: validHeaders,
      body: '{"ts":1,"type":2}\ninvalid\n{"ts":2,"data":{}}\n',
    })
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ accepted: 2 })
    expect(appendRecordingEvents).toHaveBeenCalledWith(
      {
        tabId: 101,
        documentId: '33D25F3CF060E81B14070BC356FF1871',
      },
      [
        { ts: 1, type: 2, data: undefined },
        { ts: 2, type: undefined, data: {} },
      ],
      'batch-1',
      true,
    )
  })

  test('returns accepted zero without storage for an empty valid batch', async () => {
    const appendRecordingEvents = mock(async () => ({ accepted: 1 }))
    const response = await httpTestApp({
      recordings: createRecordingHandlers({ appendRecordingEvents }),
    }).request('/api/v1/recordings/events', {
      method: 'POST',
      headers: validHeaders,
      body: '\n',
    })
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ accepted: 0 })
    expect(appendRecordingEvents).not.toHaveBeenCalled()
  })

  test('rejects invalid content type and recording identity headers', async () => {
    const appendRecordingEvents = mock(async () => ({ accepted: 1 }))
    const app = httpTestApp({
      recordings: createRecordingHandlers({ appendRecordingEvents }),
    })
    for (const headers of [
      { ...validHeaders, 'content-type': 'application/json' },
      { ...validHeaders, 'x-recording-tab-id': '0' },
      { ...validHeaders, 'x-recording-document-id': 'not-a-document' },
      { ...validHeaders, 'x-recording-batch-id': '' },
      { ...validHeaders, 'x-recording-has-gap': 'sometimes' },
    ]) {
      const response = await app.request('/api/v1/recordings/events', {
        method: 'POST',
        headers,
        body: '{"ts":1}\n',
      })
      expect(response.status).toBe(400)
      expect(await response.json()).toMatchObject({ code: 'invalid_request' })
    }
    expect(appendRecordingEvents).not.toHaveBeenCalled()
  })

  test('enforces the recording byte ceiling before append', async () => {
    const appendRecordingEvents = mock(async (_identity, events) => ({
      accepted: events.length,
    }))
    const app = httpTestApp({
      recordings: createRecordingHandlers({ appendRecordingEvents }),
    })
    const accepted = await app.request('/api/v1/recordings/events', {
      method: 'POST',
      headers: validHeaders,
      body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES, 1),
    })
    expect(accepted.status).toBe(200)

    const rejected = await app.request('/api/v1/recordings/events', {
      method: 'POST',
      headers: validHeaders,
      body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES + 1, 2),
    })
    expect(rejected.status).toBe(413)
    expect(await rejected.json()).toMatchObject({
      code: 'recording_payload_too_large',
    })
    expect(appendRecordingEvents).toHaveBeenCalledTimes(1)
  })
})

function recordingLineOfBytes(bytes: number, timestamp: number): string {
  const prefix = `{"ts":${timestamp.toString()},"type":2,"data":{"html":"`
  const suffix = '"}}'
  return `${prefix}${'x'.repeat(bytes - prefix.length - suffix.length)}${suffix}`
}
