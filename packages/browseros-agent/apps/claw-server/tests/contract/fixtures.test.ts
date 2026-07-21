/**
 * Loads every shared contract fixture through its generated DTO type. The
 * TypeScript package intentionally has no runtime serializers; HTTP behavior
 * is validated by the cross-server suite, while Rust separately deserializes
 * these same fixtures through its generated models.
 */

import { describe, expect, test } from 'bun:test'
import type {
  ApiError,
  AppendRecordingEventsResponse,
  CancelSessionResponse,
  Connection,
  ConnectionList,
  HealthResponse,
  RecordingMetadata,
  SessionDetail,
  SessionList,
  ShutdownResponse,
  SystemInfo,
  TelemetryState,
} from '@browseros/claw-api'
import { canonicalApiError } from '../../src/lib/api-error'

const fixturesDirectory = new URL(
  '../../../../contracts/claw-api/fixtures/',
  import.meta.url,
)

interface FixtureTypes {
  'health.json': HealthResponse
  'shutdown.json': ShutdownResponse
  'system-info.json': SystemInfo
  'telemetry-state.json': TelemetryState
  'session-list.json': SessionList
  'session-detail.json': SessionDetail
  'cancel-session.json': CancelSessionResponse
  'recording-metadata.json': RecordingMetadata
  'append-recording-events.json': AppendRecordingEventsResponse
  'connection.json': Connection
  'connection-list.json': ConnectionList
  'api-error.json': ApiError
  'api-error-minimal.json': ApiError
}

const fixtures = [
  'health.json',
  'shutdown.json',
  'system-info.json',
  'telemetry-state.json',
  'session-list.json',
  'session-detail.json',
  'cancel-session.json',
  'recording-metadata.json',
  'append-recording-events.json',
  'connection.json',
  'connection-list.json',
  'api-error.json',
  'api-error-minimal.json',
] as const satisfies ReadonlyArray<keyof FixtureTypes>

async function readFixture<Name extends keyof FixtureTypes>(
  file: Name,
): Promise<FixtureTypes[Name]> {
  return Bun.file(new URL(file, fixturesDirectory)).json()
}

describe('canonical contract fixtures', () => {
  for (const file of fixtures) {
    test(`loads ${file} as its generated DTO`, async () => {
      const fixture = await readFixture(file)

      expect(fixture).toBeDefined()
    })
  }

  test('canonical errors omit an unavailable request id', () => {
    expect(canonicalApiError('not_found', 'Missing')).toEqual({
      code: 'not_found',
      message: 'Missing',
    })
    expect(canonicalApiError('not_found', 'Missing', 'request-1')).toEqual({
      code: 'not_found',
      message: 'Missing',
      requestId: 'request-1',
    })
  })
})
