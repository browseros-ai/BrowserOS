/**
 * A run's OpenTelemetry trace as the screen draws it (`Waterfall`, the sidebar's Trace panel, the
 * run block's Trace row): bars on the run's own time axis, which span is selected, the selected
 * span's attributes exactly as Neo emitted them, and whether any span carries site detail (the
 * privacy check a developer makes here). Pure, read from the API's `RunTrace`.
 *
 * A span and a method row are one operation when the span's `neo.op.id` (`TraceSpan.opId`) is the
 * operation's id (`NeoOperation.id`): the same id in Neo's host record and in a record folded
 * from spans, so the waterfall and the run block can point at each other in either mode.
 */

import type {
  NeoRunRecord,
  RunSummary,
  RunTrace,
  TraceSpan,
} from '../../shared/api'
import { formatCode } from '../../shared/code-format'
import {
  callPlaces,
  type OperationSelection,
  placeOperations,
} from './operations'
import { codeLines, type Line, type Tone } from './text-lines'
import { plural, seconds } from './units'

/** The first 8 hex of a trace id, as a short commit id is read. */
export function shortTraceId(traceId: string): string {
  return traceId.slice(0, 8)
}

/** One fact about a trace, and its tone: what failed in the error tone, a partial trace in amber. */
export interface TraceFact {
  text: string
  tone: 'plain' | 'bad' | 'busy'
}

/**
 * `9 spans`, `1 failed`, `1.23 s`, and `waiting for neo.run` while it has not arrived: what a
 * trace holds, shown joined by commas wherever a trace is summed up. "Failed" counts the
 * operations whose `neo.op.outcome` is `error` (as `TraceSpan.error` does): a cancelled op is not
 * a failure, and the run's own status is the run's, not a failed span.
 */
export function traceFacts(trace: RunTrace): TraceFact[] {
  const failed = trace.spans.filter(
    (span) => span.type === 'op' && spanEnd(span) === 'failed',
  ).length
  return [
    { text: plural(trace.spans.length, 'span'), tone: 'plain' },
    ...(failed > 0 ? [{ text: `${failed} failed`, tone: 'bad' } as const] : []),
    { text: seconds(trace.durationMs), tone: 'plain' },
    ...(trace.complete
      ? []
      : [{ text: 'waiting for neo.run', tone: 'busy' } as const]),
  ]
}

/**
 * How a span ended, from what Neo says on it: an op's `neo.op.outcome`, the run's
 * `neo.run.status`; never its OTel status. A cancelled op is not a failure: Neo's
 * own `events.next` long-poll ends cancelled in every run. For the run it is the run's status (an
 * error or a timeout reads `failed`), shown on its dot only: only operations count as failed.
 */
export function spanEnd(span: TraceSpan): 'ok' | 'failed' | 'cancelled' {
  if (span.type === 'run') {
    const status = span.attributes['neo.run.status']
    if (status === undefined || status === 'ok') return 'ok'
    return status === 'cancelled' ? 'cancelled' : 'failed'
  }
  const outcome = span.attributes['neo.op.outcome']
  if (outcome === 'error') return 'failed'
  return outcome === 'cancelled' ? 'cancelled' : 'ok'
}

/**
 * A span Neo made for its own work rather than for a line of the script: an operation right
 * under the run with no source line (`page.frames`, `events.listen`, `events.next`, opening the
 * task's page). The run block folds the same calls as "Neo internal"; an operation that ran
 * inside another stays under the call it ran in.
 */
export function isInternalSpan(span: TraceSpan): boolean {
  return (
    span.type === 'op' &&
    span.depth <= 1 &&
    span.attributes['code.line.number'] === undefined
  )
}

/**
 * The spans in the waterfall's order: the run and the script's own operations first, then Neo's
 * own work (`isInternalSpan`), each with what ran inside it; draw order kept within each.
 */
export function splitSpans(spans: TraceSpan[]): {
  script: TraceSpan[]
  internal: TraceSpan[]
} {
  const script: TraceSpan[] = []
  const internal: TraceSpan[] = []
  // The depth of the internal span whose subtree is being walked, while inside one.
  let inside: number | null = null
  for (const span of spans) {
    if (inside !== null && span.depth > inside) {
      internal.push(span)
      continue
    }
    inside = isInternalSpan(span) ? span.depth : null
    ;(inside === null ? script : internal).push(span)
  }
  return { script, internal }
}

/** `events.next, page.frames ×6, events.listen ×6`: spans by name in order, repeats counted. */
export function namesWithCounts(spans: TraceSpan[]): string {
  const counts = new Map<string, number>()
  for (const span of spans)
    counts.set(spanName(span), (counts.get(spanName(span)) ?? 0) + 1)
  return [...counts]
    .map(([name, n]) => (n > 1 ? `${name} ×${n}` : name))
    .join(', ')
}

/** What a span's row is called: `neo.run`, or the op's name. */
export function spanName(span: TraceSpan): string {
  return span.type === 'run' ? span.name : span.label
}

/** A span's bar as shares (0..1) of the trace's time: where it starts and how long it lasts. */
export function spanBar(
  span: TraceSpan,
  durationMs: number,
): { left: number; width: number } {
  const total = Math.max(durationMs, 1)
  const left = clamp(span.startOffsetMs / total)
  return { left, width: Math.min(clamp(span.durationMs / total), 1 - left) }
}

function clamp(share: number): number {
  return Math.min(1, Math.max(0, Number.isFinite(share) ? share : 0))
}

/**
 * The span the panel shows: the one picked in the waterfall; else the selected call's span (a
 * method row picked in the block); else the run's own, drawn first.
 */
export function selectedSpan(
  trace: RunTrace,
  spanId: string | null,
  operationId: string | null,
): TraceSpan | null {
  return (
    (spanId ? trace.spans.find((span) => span.spanId === spanId) : null) ??
    (operationId
      ? trace.spans.find((span) => span.opId === operationId)
      : null) ??
    trace.spans[0] ??
    null
  )
}

/**
 * The call a span names, as the run block selects it (with where it sits in the script), when the
 * run's record holds it; null for `neo.run` and for an op the record does not have.
 */
export function operationFor(
  run: RunSummary,
  record: NeoRunRecord | null,
  span: TraceSpan,
): OperationSelection | null {
  if (!record || !span.opId) return null
  const operation = record.operations.find(
    (candidate) => candidate.id === span.opId,
  )
  if (!operation) return null
  const formatted = formatCode(run.code)
  const places = callPlaces(
    placeOperations(
      record,
      codeLines(formatted.text).length,
      formatted.lineOfSite,
    ),
  )
  return {
    runId: run.id,
    operationId: operation.id,
    place: places.get(operation.id) ?? { line: null, call: 1, calls: 1 },
  }
}

/**
 * How much of the page the spans carry (wire contract: `CLAW_TELEMETRY_SITE_DETAIL`): `url` when
 * any span has `url.full`, `host` when any has `url.domain`, else `none`, Neo's default, where no
 * span carries a URL or host. The attributes list shows what each span carries in full.
 */
export function siteDetail(trace: RunTrace): {
  level: 'none' | 'host' | 'url'
  text: string
} {
  const has = (key: string) =>
    trace.spans.some((span) => span.attributes[key] !== undefined)
  if (has('url.full'))
    return {
      level: 'url',
      text: 'Site detail url: spans carry the page URL (url.full), its userinfo, query and fragment stripped.',
    }
  if (has('url.domain'))
    return {
      level: 'host',
      text: 'Site detail host: spans carry the page host (url.domain), no path.',
    }
  return {
    level: 'none',
    text: 'Site detail none: no span carries a URL or host (url.*).',
  }
}

/**
 * Attributes as aligned lines (`neo.op.name  "locator.click"`), in the order Neo sent them, toned
 * as the reply is: strings quoted, an error's owner and code in the error tone, and any `url.*`
 * marked, since that is what the privacy check looks for.
 */
export function attributeLines(
  attributes: Record<string, string | number | boolean>,
): Line[] {
  const entries = Object.entries(attributes)
  const width = Math.max(0, ...entries.map(([key]) => key.length))
  return entries.map(([key, value]) => {
    const tone: Tone = key.startsWith('neo.error.')
      ? 'error'
      : key.startsWith('url.')
        ? 'note'
        : typeof value === 'string'
          ? 'string'
          : typeof value === 'boolean'
            ? 'keyword'
            : 'plain'
    return [
      { text: key.padEnd(width + 2), tone: 'key' },
      {
        text: typeof value === 'string' ? JSON.stringify(value) : String(value),
        tone,
      },
    ]
  })
}
