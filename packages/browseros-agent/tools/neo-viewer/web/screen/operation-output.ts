/**
 * What one operation gave the script, for the selection panel and "Open full". Neo's trace
 * records an operation's result as it crossed between the prelude (the SDK's JavaScript in the
 * run) and Rust, which is not always what the script's `await` got:
 *
 *   page.ariaSnapshot    the engine's tree `{ json, partial, document }`; the prelude renders the
 *                        text. For a whole page in the v1 form the next `page.snapshotPending`
 *                        records that text (and a delta call's delta as `returned`), so that is
 *                        shown. A locator's snapshot and `mode: 'default'` are rendered from the
 *                        tree with nothing recorded, so the tree is shown, saying so.
 *   the evaluate family  `{ value: <the page's serialized form>, … }`, shown as the prelude's
 *                        `parseEvaluationResultValue` decodes it (`page-value.ts`)
 *   a string             its lines
 *   anything else        the recorded result, saying that is what it is
 *
 * Formatted is that; Raw is always the recorded result as JSON. Each output carries one
 * sentence saying which it is, so the panel never passes a wire value off as the script's.
 */

import type { NeoOperation } from '../../shared/api'
import { handleText } from './operation-values'
import { isNeoInternal } from './operations'
import { pageValueText } from './page-value'
import { jsonLine, type Line, plainLines, snapshotLine } from './text-lines'

/** The calls whose result is `{ value: <the page's serialized form>, … }` (`Evaluated`). */
const EVALUATES = new Set([
  'page.evaluate',
  'worker.evaluate',
  'locator.evaluate',
  'page.evalOnSelector',
  'locator.evaluateAll',
  'page.waitForFunction',
  'locator.waitForFunction',
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
}

type Shown = Pick<
  OperationOutput,
  'formatted' | 'formattedText' | 'source' | 'cut'
>

/** `operations`: the run's, which hold the `page.snapshotPending` a snapshot's text is in. */
export function operationOutput(
  operation: NeoOperation,
  operations: NeoOperation[],
): OperationOutput {
  const rawText = recorded(operation)
  return {
    ...formatted(operation, operations),
    raw: rawText.split('\n').map(jsonLine),
    rawText,
  }
}

function recorded(operation: NeoOperation): string {
  if (operation.error) return JSON.stringify(operation.error, null, 2)
  if (operation.result === undefined) return ''
  return JSON.stringify(operation.result, null, 2) ?? 'undefined'
}

function formatted(operation: NeoOperation, operations: NeoOperation[]): Shown {
  const cut = operation.resultTruncated
  if (operation.error) {
    const text = `${operation.error.owner} / ${operation.error.code}: ${operation.error.message}`
    return {
      ...toned(text, 'error'),
      source: "The call failed: the error the script's await threw.",
      cut: false,
    }
  }
  if (operation.outcome === null) {
    return {
      ...toned('', 'muted'),
      source: 'The call has not finished.',
      cut: false,
    }
  }
  if (operation.name === 'page.ariaSnapshot') {
    const text = snapshotText(operation, operations)
    if (text) {
      return {
        ...lines(text.value),
        source:
          "The snapshot text the script's await got, as the run rendered it (recorded by Neo's page.snapshotPending).",
        cut: text.cut,
      }
    }
  }
  if (operation.name.endsWith('ariaSnapshot')) {
    return {
      ...json(operation.result),
      source: `The tree the page answered, as Neo recorded it: the run rendered the script's text from it, and that text is not recorded (a locator's snapshot, or mode: "default").`,
      cut,
    }
  }
  if (operation.name === 'page.snapshotPending' && isRecord(operation.args)) {
    const text = operation.args.returned ?? operation.args.text
    if (typeof text === 'string') {
      return {
        ...lines(text),
        source:
          "Neo internal: the snapshot text it records as this tab's pending delta baseline. The script did not call this.",
        cut: operation.argsTruncated,
      }
    }
  }
  if (EVALUATES.has(operation.name) && isRecord(operation.result)) {
    const value = operation.result.value
    const text = pageValueText(value)
    return {
      ...(typeof value === 'string' ? lines(text) : toned(text, 'json')),
      source:
        "The value the script's await got, decoded from the page's serialized form as the run decodes it.",
      cut,
    }
  }
  if (typeof operation.result === 'string') {
    return {
      ...lines(operation.result),
      source: 'The text the call returned.',
      cut,
    }
  }
  const handle = handleText(operation.result)
  if (handle !== null) {
    return {
      ...toned(handle, 'plain'),
      source: 'The handle the call returned, by what it names.',
      cut,
    }
  }
  if (operation.result === null || operation.result === undefined) {
    return {
      ...toned('(no value)', 'muted'),
      source: 'The call returned no value.',
      cut,
    }
  }
  return {
    ...json(operation.result),
    source: isNeoInternal(operation)
      ? 'Neo internal: its own record of this step. The script did not call this.'
      : "Neo's record of the result as it crossed into the run, not necessarily the value the script got: the run builds that from it.",
    cut,
  }
}

/**
 * The text a whole-page v1 snapshot gave the script: the `page.snapshotPending` that follows
 * it from the same call site, for the same document. A loop's snapshots each have their own; one
 * from another call of the same site, or none at all (`mode: 'default'`), is not this one's.
 */
function snapshotText(
  snapshot: NeoOperation,
  operations: NeoOperation[],
): { value: string; cut: boolean } | null {
  const document = isRecord(snapshot.result) ? snapshot.result.document : null
  const later = operations
    .filter((operation) => operation.seq > snapshot.seq)
    .sort((a, b) => a.seq - b.seq)
  for (const operation of later) {
    if (!sameSite(operation, snapshot)) continue
    if (operation.name === 'page.ariaSnapshot') return null
    if (operation.name !== 'page.snapshotPending' || !isRecord(operation.args))
      continue
    const { args } = operation
    if (typeof document === 'number' && typeof args.document === 'number') {
      if (args.document !== document) continue
    }
    const value = args.returned ?? args.text
    if (typeof value !== 'string') return null
    return { value, cut: operation.argsTruncated }
  }
  return null
}

function sameSite(a: NeoOperation, b: NeoOperation): boolean {
  return a.site?.line === b.site?.line && a.site?.column === b.site?.column
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
      tone === 'json'
        ? text.split('\n').map(jsonLine)
        : plainLines(text, tone === 'plain' ? 'plain' : tone),
    formattedText: text,
  }
}
