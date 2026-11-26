/** Only the selected desktop gaps (`strictFailures`) may turn their exact script assertion into an xfail. */
import { jsonObject, jsonValue, stringField } from './json'
import type {
  Expectation,
  FailureSignature,
  JsonValue,
  RunReply,
} from './model'

const ownRefPattern =
  /^Error: snapshot must give (Frame B cursor ready|Deep frame cursor ready) its own ref; actual=/
const zeroPrefix = 'Error: the zero-sized target has no actionable ref; actual='
/**
 * The frame fixture's labels in DOM stitch order. The last sits in the cross-site frame B, below
 * the fold, where APC sends no InteractionInfo (apc-oopif-actionable), so its cursor target prints
 * as text; the six before it keep their refs.
 */
const frameLabels = [
  'Parent action ready',
  'Frame A action ready',
  'Frame A cursor ready',
  'Grandchild action ready',
  'Grandchild cursor ready',
  'Frame B action ready',
  'Frame B cursor ready',
] as const
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

function plainText(snapshot: string, name: string): boolean {
  return snapshot.split('\n').some((line) => line.trim() === `- text: ${name}`)
}

/**
 * The known below-fold frame gap and nothing else: every frame label in stitch order, the six
 * in view with their refs, and the below-fold cursor targets as text. A deep frame nested in
 * frame B keeps its action ref too.
 */
function belowFoldFrameGap(snapshot: string, deep: boolean) {
  let prior = -1
  for (const label of frameLabels) {
    const index = snapshot.indexOf(label, prior + 1)
    requireSignature(
      index !== -1,
      `Expected-failure snapshot lost ${label} or its stitch order`,
    )
    prior = index
  }
  for (const label of frameLabels.slice(0, -1)) {
    requireSignature(
      refLine(snapshot, label) !== undefined,
      `Expected-failure snapshot lost ${label} ref`,
    )
  }
  requireSignature(
    plainText(snapshot, 'Frame B cursor ready'),
    'Expected-failure snapshot no longer prints Frame B cursor ready as text',
  )
  if (deep) {
    requireSignature(
      refLine(snapshot, 'Deep frame action ready') !== undefined,
      'Expected-failure snapshot lost Deep frame action ready ref',
    )
    requireSignature(
      plainText(snapshot, 'Deep frame cursor ready'),
      'Expected-failure snapshot no longer prints Deep frame cursor ready as text',
    )
  }
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
    case 'below-fold-oopif-ref': {
      const match = ownRefPattern.exec(message)
      requireSignature(
        match !== null,
        'Expected-failure assertion message changed',
      )
      const label = match[1]
      const value = jsonObject(
        payload(message, match[0]),
        'Below-fold frame expected failure',
      )
      requireSignature(
        Object.keys(value).sort().join(',') === 'line,snapshot',
        'Below-fold frame expected-failure payload fields changed',
      )
      requireSignature(
        stringField(value, 'line').trim() === `- text: ${label}`,
        `Expected-failure line for ${label} is no longer its plain text`,
      )
      belowFoldFrameGap(
        stringField(value, 'snapshot'),
        label === 'Deep frame cursor ready',
      )
      return
    }
    case 'zero-size-target-ref': {
      const value = jsonObject(
        payload(message, zeroPrefix),
        'Zero-size target expected failure',
      )
      requireSignature(
        Object.keys(value).join(',') === 'zero',
        'Zero-size target expected-failure payload fields changed',
      )
      requireSignature(
        /^- group "Zero-sized excluded target" \[ref=[^\]\s]+\] \[cursor=pointer\]$/.test(
          stringField(value, 'zero'),
        ),
        'Expected-failure zero-size line changed',
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
