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

/** `9 spans, 1 failed, 1.23 s`, and that it is still partial while `neo.run` has not arrived. */
export function traceFacts(trace: RunTrace): string {
  const failed = trace.spans.filter((span) => span.error).length
  return [
    plural(trace.spans.length, 'span'),
    ...(failed > 0 ? [`${failed} failed`] : []),
    seconds(trace.durationMs),
    ...(trace.complete ? [] : ['waiting for neo.run']),
  ].join(', ')
}

/** What a span's row is called: `neo.run`, or the op's name. */
export function spanName(span: TraceSpan): string {
  return span.name === 'neo.run' ? 'neo.run' : span.label
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
