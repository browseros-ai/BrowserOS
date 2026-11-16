/** Only the five selected desktop gaps may turn their exact script assertion into an xfail. */
import { jsonObject, jsonValue, stringField } from './json'
import type {
  Expectation,
  FailureSignature,
  JsonValue,
  RunReply,
} from './model'

const labelPrefix =
  'Error: exactly one label ref remains, for the hidden Remember me control; actual='
const isolatedPrefix =
  'Error: snapshot must give Card delete control its own ref; actual='
const cardPrefix = 'Error: No ref for Card delete control; actual='
const invalidDate =
  'Error: invalid fill leaves the field and application event record unchanged; expected={"value":"2020-01-02","events":[]}; actual={"value":"","events":[]}'

function requireSignature(
  condition: boolean,
  message: string,
): asserts condition {
  if (!condition) throw new Error(message)
}

function payload(message: string, prefix: string): JsonValue {
  requireSignature(
    message.startsWith(prefix),
    'Expected-failure assertion message changed',
  )
  // Only the numeric QuickJS source suffix is unstable. Everything before it stays
  // intact, including assertion text and JSON, so unrelated errors cannot become xfails.
  try {
    const input: unknown = JSON.parse(message.slice(prefix.length))
    return jsonValue(input)
  } catch {
    throw new Error('Expected-failure assertion payload is not valid JSON')
  }
}

function refLine(
  snapshot: string,
  name: string,
  role?: string,
): string | undefined {
  return snapshot
    .split('\n')
    .find(
      (line) =>
        line.includes(`${role ? `${role} ` : ''}"${name}"`) &&
        /\[ref=[^\]\s]+\]/.test(line),
    )
}

function missingCardChild(snapshot: string) {
  const pointer = refLine(snapshot, 'Pointer card')
  requireSignature(
    pointer?.includes('[cursor=pointer]') === true,
    'Expected-failure snapshot lost its pointer card ref',
  )
  for (const name of [
    'Assigned onclick control',
    'Listener control',
    'Shadow control',
  ]) {
    requireSignature(
      refLine(snapshot, name) !== undefined,
      `Expected-failure snapshot lost ${name} ref`,
    )
  }
  requireSignature(
    refLine(snapshot, 'Card delete control') === undefined,
    'Expected-failure snapshot now exposes the child ref',
  )
}

function matchSignature(signature: FailureSignature, message: string) {
  switch (signature) {
    case 'hidden-label-ref': {
      const value = jsonObject(
        payload(message, labelPrefix),
        'Hidden-label expected failure',
      )
      requireSignature(
        Object.keys(value).sort().join(',') === 'labelTargets,snapshot',
        'Hidden-label expected-failure payload fields changed',
      )
      requireSignature(
        Array.isArray(value.labelTargets) && value.labelTargets.length === 0,
        'Expected failure must have no label targets',
      )
      const snapshot = stringField(value, 'snapshot')
      for (const name of ['Email address', 'Wrapped name', 'Pointer label']) {
        requireSignature(
          refLine(snapshot, name, 'textbox') !== undefined,
          `Expected-failure snapshot lost ${name} textbox ref`,
        )
      }
      requireSignature(
        snapshot.includes('Remember me'),
        'Expected-failure snapshot lost Remember me text',
      )
      return
    }
    case 'isolated-child-ref': {
      const value = jsonObject(
        payload(message, isolatedPrefix),
        'Isolated-child expected failure',
      )
      requireSignature(
        Object.keys(value).join(',') === 'snapshot',
        'Isolated-child expected-failure payload fields changed',
      )
      missingCardChild(stringField(value, 'snapshot'))
      return
    }
    case 'card-child-ref': {
      const snapshot = payload(message, cardPrefix)
      requireSignature(
        typeof snapshot === 'string',
        'Card-child expected-failure payload must be a snapshot string',
      )
      missingCardChild(snapshot)
      return
    }
    case 'invalid-date-reset':
      requireSignature(
        message === invalidDate,
        'Invalid-date expected-failure payload changed',
      )
      return
    default: {
      const exhaustive: never = signature
      throw new Error(`Unimplemented expected-failure signature ${exhaustive}`)
    }
  }
}

/** Strict xfails admit only a known script assertion, never transport, host, or cleanup failures. */
export function assertExpectedFailure({
  expectation,
  reply,
}: {
  expectation: Expectation
  reply: RunReply
}): void {
  requireSignature(
    expectation.kind === 'xfail',
    'A passing expectation has no expected-failure signature',
  )
  requireSignature(
    reply.kind === 'error' && reply.isError,
    `XPASS for expected failure ${expectation.signature}`,
  )
  requireSignature(
    reply.status === 'error',
    'Expected failure must have metadata status error',
  )
  requireSignature(
    reply.error.owner === 'script',
    'Expected failure must be owned by script',
  )
  requireSignature(
    reply.error.code === 'script_error',
    'Expected failure must use code script_error',
  )
  const message = reply.error.message.replace(/ \(run:\d+:\d+\)$/, '')
  matchSignature(expectation.signature, message)
}

export function matchesExpectedFailure(args: {
  expectation: Expectation
  reply: RunReply
}): boolean {
  if (args.expectation.kind !== 'xfail') return false
  try {
    assertExpectedFailure(args)
    return true
  } catch {
    return false
  }
}
