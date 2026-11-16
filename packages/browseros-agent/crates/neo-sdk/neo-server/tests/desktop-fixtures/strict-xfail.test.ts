/** Offline signatures reject XPASS and unrelated errors before live suite classification. */
import { describe, expect, test } from 'bun:test'
import {
  type Expectation,
  type FailureSignature,
  isRunId,
  type RunError,
  type RunReply,
} from './model'
import { assertExpectedFailure, matchesExpectedFailure } from './strict-xfail'

const labelSnapshot = [
  '- textbox "Email address" [ref=first]',
  '- textbox "Wrapped name" [ref=second]',
  '- textbox "Pointer label" [ref=third]',
  '- text: Remember me',
].join('\n')
const cardSnapshot = [
  '- heading "Script isolation fixture" [level=1]',
  '- group "Assigned onclick control" [ref=first] [cursor=pointer]',
  '- group "Listener control" [ref=second] [cursor=pointer]',
  '- group "Shadow control" [ref=third] [cursor=pointer]',
  '- group "Pointer card" [ref=fourth] [cursor=pointer]',
  '- status: untouched',
].join('\n')
const labelPrefix =
  'Error: exactly one label ref remains, for the hidden Remember me control; actual='
const isolatedPrefix =
  'Error: snapshot must give Card delete control its own ref; actual='
const cardPrefix = 'Error: No ref for Card delete control; actual='
const invalidDate =
  'Error: invalid fill leaves the field and application event record unchanged; expected={"value":"2020-01-02","events":[]}; actual={"value":"","events":[]}'

function expectation(signature: FailureSignature): Expectation {
  return { kind: 'xfail', reason: 'Known desktop contract gap.', signature }
}

function failure(
  message: string,
  overrides: Partial<RunError> = {},
  status = 'error',
): RunReply {
  const runId = '11111111-2222-3333-4444-555555555555'
  if (!isRunId(runId)) throw new Error('Invalid test run ID')
  return {
    kind: 'error',
    isError: true,
    runId,
    status,
    text: message,
    images: [],
    raw: {},
    error: { owner: 'script', code: 'script_error', message, ...overrides },
  }
}

const knownMessages: readonly {
  signature: FailureSignature
  message: string
}[] = [
  {
    signature: 'hidden-label-ref',
    message:
      labelPrefix +
      JSON.stringify({ labelTargets: [], snapshot: labelSnapshot }),
  },
  {
    signature: 'isolated-child-ref',
    message: isolatedPrefix + JSON.stringify({ snapshot: cardSnapshot }),
  },
  {
    signature: 'card-child-ref',
    message: cardPrefix + JSON.stringify(cardSnapshot),
  },
  { signature: 'invalid-date-reset', message: invalidDate },
]

describe('strict desktop expected failures', () => {
  test('accepts every known payload and normalizes only numeric run source suffixes', () => {
    for (const known of knownMessages) {
      for (const suffix of ['', ' (run:5:103)', ' (run:9876:12345)']) {
        expect(
          matchesExpectedFailure({
            expectation: expectation(known.signature),
            reply: failure(known.message + suffix),
          }),
        ).toBe(true)
      }
    }
  })

  test('rejects wrong owners, codes, and metadata statuses for every known payload', () => {
    for (const known of knownMessages) {
      const expected = expectation(known.signature)
      for (const owner of ['host', 'neo', 'website']) {
        expect(
          matchesExpectedFailure({
            expectation: expected,
            reply: failure(known.message, { owner }),
          }),
        ).toBe(false)
      }
      for (const code of [
        'wait_timeout',
        'invalid_argument',
        'unknown_method',
      ]) {
        expect(
          matchesExpectedFailure({
            expectation: expected,
            reply: failure(known.message, { code }),
          }),
        ).toBe(false)
      }
      expect(
        matchesExpectedFailure({
          expectation: expected,
          reply: failure(known.message, {}, 'ok'),
        }),
      ).toBe(false)
      expect(
        matchesExpectedFailure({
          expectation: expected,
          reply: failure(known.message, {}, 'running'),
        }),
      ).toBe(false)
    }
  })

  test('rejects XPASS and provides a concrete diagnostic', () => {
    const runId = '11111111-2222-3333-4444-555555555555'
    if (!isRunId(runId)) throw new Error('Invalid test run ID')
    const reply: RunReply = {
      kind: 'ok',
      isError: false,
      runId,
      status: 'ok',
      value: null,
      text: '',
      images: [],
      raw: {},
    }
    const args = { expectation: expectation('invalid-date-reset'), reply }
    expect(matchesExpectedFailure(args)).toBe(false)
    expect(() => assertExpectedFailure(args)).toThrow('XPASS')
    expect(
      matchesExpectedFailure({
        expectation: { kind: 'pass' },
        reply: failure(invalidDate),
      }),
    ).toBe(false)
  })

  test('does not normalize changed assertion text or nonnumeric stack suffixes', () => {
    for (const known of knownMessages) {
      for (const message of [
        `${known.message} (run:x:103)`,
        `${known.message} (other:5:103)`,
        `${known.message}\n at run:5:103`,
        `Error: earlier assertion; ${known.message}`,
      ]) {
        expect(
          matchesExpectedFailure({
            expectation: expectation(known.signature),
            reply: failure(message),
          }),
        ).toBe(false)
      }
    }
  })

  test('hidden labels require all three textbox refs, Remember me text, and no label targets', () => {
    const expected = expectation('hidden-label-ref')
    const payloads = [
      {
        labelTargets: [{ ref: 'new', text: 'Remember me' }],
        snapshot: labelSnapshot,
      },
      {
        labelTargets: [],
        snapshot: labelSnapshot.replace(
          'textbox "Wrapped name" [ref=second]',
          'textbox "Wrapped name"',
        ),
      },
      {
        labelTargets: [],
        snapshot: labelSnapshot.replace('Remember me', 'Other label'),
      },
      { labelTargets: [], snapshot: labelSnapshot, unrelated: true },
      { snapshot: labelSnapshot },
    ]
    for (const value of payloads) {
      expect(
        matchesExpectedFailure({
          expectation: expected,
          reply: failure(labelPrefix + JSON.stringify(value)),
        }),
      ).toBe(false)
    }
    expect(
      matchesExpectedFailure({
        expectation: expected,
        reply: failure(`${labelPrefix}{broken json`),
      }),
    ).toBe(false)
  })

  test('isolation signatures require the pointer card, three other refs, and no child ref', () => {
    const wrongSnapshots = [
      cardSnapshot.replace(
        '"Pointer card" [ref=fourth] [cursor=pointer]',
        '"Pointer card" [ref=fourth]',
      ),
      cardSnapshot.replace(
        '"Listener control" [ref=second]',
        '"Listener control"',
      ),
      cardSnapshot.replace(
        '"Shadow control" [ref=third]',
        '"Different control" [ref=third]',
      ),
      `${cardSnapshot}\n- button "Card delete control" [ref=fixed]`,
    ]
    for (const snapshot of wrongSnapshots) {
      expect(
        matchesExpectedFailure({
          expectation: expectation('isolated-child-ref'),
          reply: failure(isolatedPrefix + JSON.stringify({ snapshot })),
        }),
      ).toBe(false)
      expect(
        matchesExpectedFailure({
          expectation: expectation('card-child-ref'),
          reply: failure(cardPrefix + JSON.stringify(snapshot)),
        }),
      ).toBe(false)
    }
    expect(
      matchesExpectedFailure({
        expectation: expectation('isolated-child-ref'),
        reply: failure(
          isolatedPrefix +
            JSON.stringify({ snapshot: cardSnapshot, line: 'different' }),
        ),
      }),
    ).toBe(false)
    expect(
      matchesExpectedFailure({
        expectation: expectation('card-child-ref'),
        reply: failure(cardPrefix + JSON.stringify({ snapshot: cardSnapshot })),
      }),
    ).toBe(false)
  })

  test('invalid dates match the entire original and changed event/value payload', () => {
    for (const message of [
      invalidDate.replace('"value":""', '"value":"2020-01-02"'),
      invalidDate.replace(
        'actual={"value":"","events":[]}',
        'actual={"value":"","events":["input"]}',
      ),
      `${invalidDate} extra`,
    ]) {
      expect(
        matchesExpectedFailure({
          expectation: expectation('invalid-date-reset'),
          reply: failure(message),
        }),
      ).toBe(false)
    }
    expect(() =>
      assertExpectedFailure({
        expectation: expectation('invalid-date-reset'),
        reply: failure(invalidDate, { owner: 'host' }),
      }),
    ).toThrow('owned by script')
    expect(() =>
      assertExpectedFailure({
        expectation: expectation('invalid-date-reset'),
        reply: failure(invalidDate, { code: 'wait_timeout' }),
      }),
    ).toThrow('code script_error')
  })
})
