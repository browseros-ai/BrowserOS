/**
 * Round-trips every shared contract fixture through the generated
 * deserializers and back, asserting the JSON survives value-for-value.
 * The same fixtures feed the Rust server's `contract_fixtures` tests,
 * so a fixture that survives both proves the two type systems agree on
 * the wire shape.
 */

import { describe, expect, test } from 'bun:test'
import {
  ApiErrorFromJSON,
  AppendRecordingEventsResponseFromJSON,
  CancelSessionResponseFromJSON,
  ConnectionFromJSON,
  ConnectionListFromJSON,
  HealthResponseFromJSON,
  RecordingMetadataFromJSON,
  SessionDetailFromJSON,
  SessionListFromJSON,
  ShutdownResponseFromJSON,
  SystemInfoFromJSON,
  TelemetryStateFromJSON,
} from '@browseros/claw-api'
import { canonicalApiError } from '../../src/lib/api-error'

const fixturesDirectory = new URL(
  '../../../../contracts/claw-api/fixtures/',
  import.meta.url,
)

const fixtures = [
  ['health.json', HealthResponseFromJSON],
  ['shutdown.json', ShutdownResponseFromJSON],
  ['system-info.json', SystemInfoFromJSON],
  ['telemetry-state.json', TelemetryStateFromJSON],
  ['session-list.json', SessionListFromJSON],
  ['session-detail.json', SessionDetailFromJSON],
  ['cancel-session.json', CancelSessionResponseFromJSON],
  ['recording-metadata.json', RecordingMetadataFromJSON],
  ['append-recording-events.json', AppendRecordingEventsResponseFromJSON],
  ['connection.json', ConnectionFromJSON],
  ['connection-list.json', ConnectionListFromJSON],
  ['api-error.json', ApiErrorFromJSON],
  ['api-error-minimal.json', ApiErrorFromJSON],
] as const

describe('canonical contract fixtures', () => {
  for (const [file, fromJson] of fixtures) {
    test(`deserializes ${file} with generated DTOs`, async () => {
      const fixture = await Bun.file(new URL(file, fixturesDirectory)).json()
      const parsed = fromJson(fixture)

      expect(JSON.parse(JSON.stringify(parsed))).toEqual(fixture)
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
