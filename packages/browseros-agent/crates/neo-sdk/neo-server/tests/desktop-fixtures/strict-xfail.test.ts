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

const frameSnapshot = [
  '- heading "Snapshot frame tree fixture" [level=1]',
  '- button "Parent action ready" [ref=e3]',
  '- iframe [ref=e4]:',
  '  - button "Frame A action ready" [ref=f1e2]',
  '  - group "Frame A cursor ready" [ref=f1e3] [cursor=pointer]',
  '  - iframe [ref=f1e4]:',
  '    - button "Grandchild action ready" [ref=f3e2]',
  '    - group "Grandchild cursor ready" [ref=f3e3] [cursor=pointer]',
  '- iframe [ref=e5]:',
  '  - button "Frame B action ready" [ref=f2e1]',
  '  - text: Frame B cursor ready',
].join('\n')
const deepSnapshot = [
  frameSnapshot,
  '  - iframe [ref=f2e3]:',
  '    - button "Deep frame action ready" [ref=f4e1]',
  '    - text: Deep frame cursor ready',
].join('\n')
const zeroLine =
  '- group "Zero-sized excluded target" [ref=e71] [cursor=pointer]'
const cardSnapshot = [
  '- heading "Script isolation fixture" [level=1]',
  '- group "Assigned onclick control" [ref=first] [cursor=pointer]',
  '- group "Listener control" [ref=second] [cursor=pointer]',
  '- group "Shadow control" [ref=third] [cursor=pointer]',
  '- group "Pointer card" [ref=fourth] [cursor=pointer]',
  '- status: untouched',
].join('\n')
const frameBPrefix =
  'Error: snapshot must give Frame B cursor ready its own ref; actual='
const deepPrefix =
  'Error: snapshot must give Deep frame cursor ready its own ref; actual='
const zeroPrefix = 'Error: the zero-sized target has no actionable ref; actual='
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
    signature: 'below-fold-oopif-ref',
    message:
      frameBPrefix +
      JSON.stringify({
        line: '  - text: Frame B cursor ready',
        snapshot: frameSnapshot,
      }),
  },
  {
    signature: 'below-fold-oopif-ref',
    message:
      deepPrefix +
      JSON.stringify({
        line: '    - text: Deep frame cursor ready',
        snapshot: deepSnapshot,
      }),
  },
  {
    signature: 'zero-size-target-ref',
    message: zeroPrefix + JSON.stringify({ zero: zeroLine }),
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

  test('below-fold frame signatures require stitch order, the six in-view refs, and only the cursor as text', () => {
    const wrongSnapshots = [
      frameSnapshot.replace(
        '- text: Frame B cursor ready',
        '- group "Frame B cursor ready" [ref=f2e2] [cursor=pointer]',
      ),
      frameSnapshot.replace(
        'group "Frame A cursor ready" [ref=f1e3] [cursor=pointer]',
        'text: Frame A cursor ready',
      ),
      frameSnapshot.replace(
        'button "Frame B action ready" [ref=f2e1]',
        'button "Frame B action ready"',
      ),
      frameSnapshot
        .replace('Parent action ready', 'Moved action')
        .concat('\n- button "Parent action ready" [ref=e9]'),
    ]
    for (const snapshot of wrongSnapshots) {
      expect(
        matchesExpectedFailure({
          expectation: expectation('below-fold-oopif-ref'),
          reply: failure(
            frameBPrefix +
              JSON.stringify({
                line: '  - text: Frame B cursor ready',
                snapshot,
              }),
          ),
        }),
      ).toBe(false)
    }
    expect(
      matchesExpectedFailure({
        expectation: expectation('below-fold-oopif-ref'),
        reply: failure(
          deepPrefix +
            JSON.stringify({
              line: '    - text: Deep frame cursor ready',
              snapshot: deepSnapshot.replace(
                'button "Deep frame action ready" [ref=f4e1]',
                'button "Deep frame action ready"',
              ),
            }),
        ),
      }),
    ).toBe(false)
    expect(
      matchesExpectedFailure({
        expectation: expectation('below-fold-oopif-ref'),
        reply: failure(
          frameBPrefix +
            JSON.stringify({
              line: '  - text: Frame B cursor ready',
              snapshot: frameSnapshot,
              extra: 1,
            }),
        ),
      }),
    ).toBe(false)
  })

  test('the zero-size signature requires exactly the zero-sized group line with a ref', () => {
    for (const zero of [
      '- group "Zero-sized excluded target"',
      '- group "Other target" [ref=e71] [cursor=pointer]',
      zeroLine.replace(' [cursor=pointer]', ''),
    ]) {
      expect(
        matchesExpectedFailure({
          expectation: expectation('zero-size-target-ref'),
          reply: failure(zeroPrefix + JSON.stringify({ zero })),
        }),
      ).toBe(false)
    }
    expect(
      matchesExpectedFailure({
        expectation: expectation('zero-size-target-ref'),
        reply: failure(
          zeroPrefix + JSON.stringify({ zero: zeroLine, other: 1 }),
        ),
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
