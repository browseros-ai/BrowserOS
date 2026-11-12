/**
 * What one operation gave the script, for the selection panel and "Open full". Neo's trace
 * records an operation's result as it crossed between the prelude (the SDK's JavaScript in the
 * run) and Rust, which is not always what the script's `await` got (frame.ts, evaluation.ts):
 *
 *   page.ariaSnapshot       the engine's tree `{ json, partial, document }`. In the v1 form the
 *                           run renders the text, and the whole page's `page.snapshotPending`
 *                           that follows records it (a delta call's delta as `returned`); the
 *                           script got it only if that operation succeeded.
 *   mode: 'default'         Playwright's YAML, rebuilt from the recorded tree as the prelude's
 *                           `renderSnapshot` builds it (`snapshot-yaml.ts`), unread frames' comments
 *                           included.
 *   locator.ariaSnapshot    v1 text rendered with options the wire does not carry: the tree,
 *                           saying the text was not recorded.
 *   page.evaluate           `{ value, frames }`; the evaluate methods below it the serialized
 *                           value itself; page.waitForFunction `{ value, preview }`, which the
 *                           script gets as a JSHandle. Each decoded as the run decodes it
 *                           (`page-value.ts`).
 *   a string                its lines
 *   anything else           the recorded result, saying that is what it is
 *
 * Formatted is that; Raw is always the recorded result as JSON. Each output carries one sentence
 * saying which it is, so the panel never passes a wire value off as the script's.
 */

import type { AriaSnapshotJSON } from '../../../../crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
import type { NeoOperation, NeoOperationError } from '../../shared/api'
import { handleText } from './operation-values'
import { isNeoInternal } from './operations'
import { pageValueText } from './page-value'
import { defaultSnapshotText, type PartialFrame } from './snapshot-yaml'
import { jsonLine, type Line, plainLines, snapshotLine } from './text-lines'

/** The evaluate methods whose result is the page's serialized value itself. */
const SERIALIZED = new Set([
  'worker.evaluate',
  'locator.evaluate',
  'page.evalOnSelector',
  'locator.evaluateAll',
])

export interface OperationOutput {
  /** What the script got, or the recorded result where that cannot be recovered. */
  formatted: Line[]
  /** The text `formatted` shows; its size is the one the panel gives. */
  formattedText: string
  /** The recorded result, as JSON. */
  raw: Line[]
  rawText: string
  /** One sentence: what Formatted shows, and where it comes from. */
  source: string
  /** Neo kept only the start of what Formatted shows. */
  cut: boolean
  /**
   * Neo kept only the start of the recorded result Raw shows. Separate from `cut`: a snapshot's
   * text comes from its pending record's arguments, which can be whole while its tree was cut.
   */
  rawCut: boolean
}

type Shown = Pick<
  OperationOutput,
  'formatted' | 'formattedText' | 'source' | 'cut'
>

/**
 * `operations`: the run's, which hold the `page.snapshotPending` a snapshot's text is in;
 * `traceGaps`: how many of its records Neo dropped, past which no text is paired.
 */
export function operationOutput(
  operation: NeoOperation,
  operations: NeoOperation[],
  traceGaps = 0,
): OperationOutput {
  const rawText = recorded(operation)
  return {
    ...shownOrRecord(operation, traceGaps > 0 ? null : operations),
    raw: rawText.split('\n').map(jsonLine),
    rawText,
    rawCut: operation.error ? false : operation.resultTruncated,
  }
}

/**
 * The first reading that applies. One that fails (a value no reading expected) falls back to
 * Neo's record: a run's block computes this for its open call, and must never break on it.
 */
function shownOrRecord(
  operation: NeoOperation,
  operations: NeoOperation[] | null,
): Shown {
  try {
    return (
      failed(operation) ??
      unfinished(operation) ??
      cancelled(operation) ??
      snapshot(operation, operations) ??
      pending(operation) ??
      evaluated(operation) ??
      recordedValue(operation)
    )
  } catch {
    return tree(
      operation,
      "The viewer could not read this result, so this is Neo's record of it.",
    )
  }
}

function recorded(operation: NeoOperation): string {
  if (operation.error) return JSON.stringify(operation.error, null, 2)
  if (operation.result === undefined) return ''
  return JSON.stringify(operation.result, null, 2) ?? 'undefined'
}

function failed(operation: NeoOperation): Shown | null {
  if (!operation.error) return null
  return thrown(operation.error, failureSource(operation))
}

/**
 * Who got an operation's failure. The snapshot's bookkeeping is awaited by the snapshot call,
 * which throws what it threw; the event pump's failures the run handles itself.
 */
function failureSource(operation: NeoOperation): string {
  if (operation.name.startsWith('page.snapshot')) {
    return `Neo internal: the snapshot's bookkeeping failed, and the script's ariaSnapshot() await threw this.`
  }
  if (isNeoInternal(operation)) {
    return 'Neo internal: its event bookkeeping failed. The script did not call this; the run handles that failure itself.'
  }
  return "The call failed: the error the script's await threw."
}

function unfinished(operation: NeoOperation): Shown | null {
  if (operation.outcome !== null) return null
  return {
    ...toned('', 'muted'),
    source: 'The call has not finished.',
    cut: false,
  }
}

/** A call stopped before it answered: it returned nothing, whatever its record holds. */
function cancelled(operation: NeoOperation): Shown | null {
  if (operation.outcome !== 'cancelled') return null
  return {
    ...toned('(cancelled)', 'muted'),
    source: isNeoInternal(operation)
      ? 'Neo internal: stopped when the run ended or stopped waiting for it; it returned no value.'
      : 'The call was cancelled before it answered, so it returned no value.',
    cut: false,
  }
}

/** `operations` is null when Neo dropped records: a snapshot's text is then not paired. */
function snapshot(
  operation: NeoOperation,
  operations: NeoOperation[] | null,
): Shown | null {
  if (!operation.name.endsWith('.ariaSnapshot')) return null
  const result = operation.result
  // ariaSnapshotJSON() runs the same operation and returns the tree itself.
  if (apiOf(operation).endsWith('ariaSnapshotJSON')) {
    return {
      ...json(isRecord(result) ? result.json : result),
      source:
        "The tree the script's await got: ariaSnapshotJSON() returns the page's JSON itself.",
      cut: operation.resultTruncated,
    }
  }
  const text =
    rendered(operation) ??
    (operation.name === 'page.ariaSnapshot'
      ? operations
        ? textOf(operation, operations)
        : tree(
            operation,
            "The tree the page answered, as Neo recorded it: Neo dropped some of this run's records, so which recorded text was this call's cannot be told.",
          )
      : null)
  return (
    text ??
    tree(
      operation,
      "The tree the page answered, as Neo recorded it. The run rendered the script's text from it, and that text is not recorded here.",
    )
  )
}

/** `mode: 'default'` with every frame read: the run's own YAML renderer on the recorded tree. */
function rendered(operation: NeoOperation): Shown | null {
  const result = operation.result
  if (modeOf(operation) !== 'default' || operation.resultTruncated) return null
  if (!isSnapshotResult(result)) return null
  return {
    ...lines(defaultSnapshotText(result.json, result.partial)),
    source:
      "The YAML the script's await got (mode: \"default\"), rendered from Neo's recorded tree by the run's own renderer.",
    cut: false,
  }
}

/** The recorded tree, with the sentence saying why it is shown instead of the text. */
function tree(operation: NeoOperation, source: string): Shown {
  return { ...json(operation.result), source, cut: operation.resultTruncated }
}

/**
 * A whole-page v1 snapshot's text: the `page.snapshotPending` it recorded, found by walking the
 * operations of the same tab and call site in order. Each v1 snapshot opens; a pending record
 * for the same document closes the one snapshot open. When two are open (calls that overlapped),
 * either could own it, so neither is given a text: the tree is shown, saying so. When the
 * pending operation failed, the script's await threw instead.
 */
function textOf(
  snapshot: NeoOperation,
  operations: NeoOperation[],
): Shown | null {
  const match = pendingOf(snapshot, operations)
  if (match !== 'overlapped') return match ? pendingText(match) : null
  return tree(
    snapshot,
    "The tree the page answered, as Neo recorded it: snapshots of this tab from this line overlapped, so Neo's record cannot tell which text was this call's.",
  )
}

function pendingOf(
  snapshot: NeoOperation,
  operations: NeoOperation[],
): NeoOperation | 'overlapped' | null {
  const call = operations
    .filter((operation) => sameCall(operation, snapshot))
    .sort((a, b) => a.seq - b.seq)
  let open: NeoOperation[] = []
  // Snapshots whose text could be either of two pending records. They stay ambiguous, and leave
  // `open` one per record, so a later snapshot from the same line is paired again.
  const overlapped = new Set<NeoOperation>()
  for (const operation of call) {
    if (recordsText(operation)) open.push(operation)
    if (!closesSnapshot(operation)) continue
    const owners = open.filter(
      (candidate) => documentOf(candidate) === argsOf(operation).document,
    )
    if (owners.length > 1) for (const owner of owners) overlapped.add(owner)
    const [closed] = owners
    if (!closed) continue
    if (owners.length === 1 && closed === snapshot && !overlapped.has(snapshot))
      return operation
    open = open.filter((candidate) => candidate !== closed)
  }
  return overlapped.has(snapshot) ? 'overlapped' : null
}

/**
 * A snapshot that records its text with a `page.snapshotPending`: a whole page's v1 text that the
 * engine answered. `mode: 'default'`, `ariaSnapshotJSON()` and a failed call never do.
 */
function recordsText(operation: NeoOperation): boolean {
  return (
    operation.name === 'page.ariaSnapshot' &&
    operation.outcome === 'ok' &&
    modeOf(operation) !== 'default' &&
    !apiOf(operation).endsWith('ariaSnapshotJSON')
  )
}

function modeOf(operation: NeoOperation): unknown {
  return argsOf(operation).mode
}

function documentOf(operation: NeoOperation): unknown {
  return isRecord(operation.result) ? operation.result.document : undefined
}

/** The API the script called, which the recorded name can differ from (`call.api`). */
function apiOf(operation: NeoOperation): string {
  const call = argsOf(operation).call
  return isRecord(call) && typeof call.api === 'string'
    ? call.api
    : operation.name
}

function argsOf(operation: NeoOperation): Record<string, unknown> {
  return isRecord(operation.args) ? operation.args : {}
}

/** What the script got from the snapshot whose text `match` recorded. */
/**
 * What ends a snapshot call after the engine answered: the pending record of its text, or for a
 * delta call a failed read of the stored baseline (the prelude throws before recording any text).
 */
function closesSnapshot(operation: NeoOperation): boolean {
  if (operation.name === 'page.snapshotPending') return true
  return (
    operation.name === 'page.snapshotBaseline' && operation.outcome !== 'ok'
  )
}

function pendingText(match: NeoOperation): Shown | null {
  if (match.error) {
    return thrown(
      match.error,
      `The snapshot was taken, but ${match.name === 'page.snapshotBaseline' ? "reading this tab's stored baseline for the delta" : 'recording its text'} failed (Neo's ${match.name}), so the script's await threw this.`,
    )
  }
  if (match.outcome !== 'ok') {
    return {
      ...toned(match.outcome === null ? '' : `(${match.outcome})`, 'muted'),
      source:
        match.outcome === null
          ? 'The snapshot is still recording its text.'
          : "The snapshot's bookkeeping did not finish, so the script's await got no text.",
      cut: false,
    }
  }
  const args = isRecord(match.args) ? match.args : {}
  const value = args.returned ?? args.text
  if (typeof value !== 'string') return null
  return {
    ...lines(value),
    source:
      "The snapshot text the script's await got, as the run rendered it (recorded by Neo's page.snapshotPending).",
    cut: match.argsTruncated && isCutText(value),
  }
}

function pending(operation: NeoOperation): Shown | null {
  if (operation.name !== 'page.snapshotPending' || !isRecord(operation.args))
    return null
  const text = operation.args.text
  if (typeof text !== 'string') return null
  return {
    ...lines(text),
    source:
      "Neo internal: the snapshot text it records as this tab's pending delta baseline. The script did not call this.",
    cut: operation.argsTruncated && isCutText(text),
  }
}

function evaluated(operation: NeoOperation): Shown | null {
  const result = operation.result
  const decodedFrom = (value: unknown, source: string): Shown =>
    decoded(operation, value, source)
  const got =
    "The value the script's await got, decoded from the page's serialized form as the run decodes it."
  if (operation.name === 'page.evaluate' && isRecord(result))
    return decodedFrom(result.value, got)
  if (SERIALIZED.has(operation.name)) return decodedFrom(result, got)
  if (operation.name === 'page.waitForFunction' && isRecord(result)) {
    return decodedFrom(
      result.value,
      "The script's await got a JSHandle: this is the value it holds, decoded as the run decodes it.",
    )
  }
  return null
}

/**
 * A serialized value decoded by the run's own decoder. A value Neo cut is not decoded: its cut
 * markers are not the page's forms, so the decoder would build a different value.
 */
function decoded(
  operation: NeoOperation,
  value: unknown,
  source: string,
): Shown {
  if (operation.resultTruncated) {
    return tree(
      operation,
      "Neo kept only the start of this value, and a cut value cannot be decoded: this is Neo's record of it.",
    )
  }
  const text = pageValueText(value)
  if (text === null) {
    return tree(
      operation,
      "The run's decoder refuses this recorded value, so this is Neo's record of it.",
    )
  }
  return {
    ...(typeof value === 'string' ? lines(text) : toned(text, 'json')),
    source:
      depthOf(value) >= RUN_DECODE_DEPTH
        ? `${source} It is nested about ${RUN_DECODE_DEPTH} levels deep or more, past what the run's decoder reaches, so the script's await may have thrown instead.`
        : source,
    cut: false,
  }
}

/**
 * The run decodes a value on a small stack: one nested more than about this deep fails its call
 * after the page answered (neo-server's README, "Maximum call stack size exceeded"), so the
 * viewer, which decodes it fine, must not say the script got it.
 */
const RUN_DECODE_DEPTH = 50

/** How deeply the page's serialized form nests its arrays, objects, Maps and Sets. */
function depthOf(value: unknown): number {
  if (!isRecord(value)) return 0
  const items = [value.a, value.o, value.m, value.s].find(Array.isArray)
  if (!items) return 0
  let deepest = 0
  for (const item of items) {
    const inner = isRecord(item) && 'k' in item ? [item.k, item.v] : [item]
    for (const part of inner) deepest = Math.max(deepest, depthOf(part))
  }
  return deepest + 1
}

/** Neo cut this string: it ends in the trace's `…[+N bytes]` marker. */
function isCutText(text: string): boolean {
  return /…\[\+\d+ bytes\]$/.test(text)
}

function recordedValue(operation: NeoOperation): Shown {
  const result = operation.result
  const cut = operation.resultTruncated
  if (isNeoInternal(operation)) {
    return {
      ...json(result),
      source:
        'Neo internal: its own record of this step. The script did not call this, and did not get this value.',
      cut,
    }
  }
  if (typeof result === 'string') {
    return { ...lines(result), source: 'The text the call returned.', cut }
  }
  const handle = handleText(result)
  if (handle !== null) {
    return {
      ...toned(handle, 'plain'),
      source: 'The handle the call returned, by what it names.',
      cut,
    }
  }
  if (result === undefined) {
    return {
      ...toned('(no value)', 'muted'),
      source: 'The call returned no value.',
      cut,
    }
  }
  if (result === null) {
    return {
      ...toned('null', 'json'),
      source:
        'Neo recorded null: the script got null, or nothing at all from a method that returns no value.',
      cut,
    }
  }
  return {
    ...json(result),
    source: isNeoInternal(operation)
      ? 'Neo internal: its own record of this step. The script did not call this.'
      : "Neo's record of the result as it crossed into the run, not necessarily the value the script got: the run builds that from it.",
    cut,
  }
}

function thrown(error: NeoOperationError, source: string): Shown {
  return {
    ...toned(`${error.owner} / ${error.code}: ${error.message}`, 'error'),
    source,
    cut: false,
  }
}

/** The same tab and the same script call site: the snapshot's own bookkeeping, not another's. */
function sameCall(a: NeoOperation, b: NeoOperation): boolean {
  return (
    a.tabId === b.tabId &&
    a.site?.line === b.site?.line &&
    a.site?.column === b.site?.column
  )
}

function isSnapshotResult(
  value: unknown,
): value is { json: AriaSnapshotJSON; partial: PartialFrame[] } {
  return (
    isRecord(value) &&
    Array.isArray(value.json) &&
    Array.isArray(value.partial) &&
    value.partial.every(
      (frame) =>
        isRecord(frame) &&
        typeof frame.ref === 'string' &&
        typeof frame.frame === 'string' &&
        typeof frame.reason === 'string',
    )
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function lines(text: string): Pick<Shown, 'formatted' | 'formattedText'> {
  return { formatted: text.split('\n').map(snapshotLine), formattedText: text }
}

function json(value: unknown): Pick<Shown, 'formatted' | 'formattedText'> {
  const text = JSON.stringify(value, null, 2) ?? 'undefined'
  return { formatted: text.split('\n').map(jsonLine), formattedText: text }
}

function toned(
  text: string,
  tone: 'error' | 'muted' | 'plain' | 'json',
): Pick<Shown, 'formatted' | 'formattedText'> {
  return {
    formatted:
      tone === 'json' ? text.split('\n').map(jsonLine) : plainLines(text, tone),
    formattedText: text,
  }
}
