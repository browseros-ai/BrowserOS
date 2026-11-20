/**
 * The W3C trace context of a run through the proxy: decided once per HTTP exchange, before the
 * request is forwarded. `mcp-proxy.ts` stores the context on the run (`RunSummary.trace`) and
 * adds the `traceparent` header this returns, if any; the body is still forwarded byte for byte.
 *
 * Why the viewer adds one at all: Neo parents its `neo.run` span under the caller's trace context
 * (the header wins over `_meta["com.browseros.neo/run"].traceparent`, wire-contract.md). With a
 * context the viewer minted, the run's spans hang under a span id that belongs to that one run,
 * so even a run whose reply never came (cancelled, Neo's reply lost) can find its `neo.run` span.
 *
 * Rules, in order:
 * - the request has a `traceparent` header: the caller's. A valid one is the run's context; an
 *   invalid one (Neo ignores it and starts a new trace) leaves the run with none. Nothing is added.
 * - else the request's one `run` call carries `_meta[…].traceparent`: the caller's, the same
 *   way. Nothing is added: a header would override it.
 * - else the request holds exactly one `run` call: the viewer mints a sampled context
 *   (`00-<trace>-<span>-01`) and adds it as the header. Sampled, or Neo's parent-based sampler
 *   would drop every span of the run.
 * - else (no call, or a batch of several): none, and nothing is added. One header cannot name
 *   several runs, and a minted span id must bind exactly one.
 * The viewer never edits what the caller sent, and never touches `tracestate`.
 */

import { randomFillSync } from 'node:crypto'
import type { RunTraceContext } from '../../shared/api'
import { isRunCall, type RequestMessage, runMetaOf } from './request-body'

export interface TraceDecision {
  /** Stored on the run start (persisted) and shown as `RunSummary.trace`. */
  context: RunTraceContext
  /** The header to add to the forwarded request; null when the caller sent its own context. */
  inject: { traceparent: string } | null
}

/** W3C Trace Context Level 1: version, trace id, parent (span) id, flags; lowercase hex only. */
const TRACEPARENT =
  /^([0-9a-f]{2})-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})(-.*)?$/

export function traceContextFor(
  headers: Headers,
  messages: readonly RequestMessage[],
  random: (bytes: number) => Uint8Array = randomBytes,
): TraceDecision | null {
  const header = headers.get('traceparent')
  if (header !== null) return caller(header)
  // Neo serves one tool, `run`; a call of any other is refused before a run (and a span) exists.
  const calls = messages.filter(
    (message) => isRunCall(message) && message.hasId,
  )
  if (calls.length !== 1) return null
  const meta = runMetaOf(calls[0] as RequestMessage)
  if (meta && 'traceparent' in meta)
    return caller(typeof meta.traceparent === 'string' ? meta.traceparent : '')
  const traceId = nonZeroHex(random, 16)
  const spanId = nonZeroHex(random, 8)
  return {
    context: { traceId, spanId, injected: true },
    inject: { traceparent: `00-${traceId}-${spanId}-01` },
  }
}

/** The caller's own `traceparent`: its context when Neo would accept it, never an injection. */
function caller(value: string): TraceDecision | null {
  const parsed = parseTraceparent(value)
  return parsed
    ? { context: { ...parsed, injected: false }, inject: null }
    : null
}

/**
 * A `traceparent` as Neo's propagator reads it, or null when Neo would ignore it: version `ff`,
 * an all-zero id, uppercase hex, or a version-00 value with anything after its flags. A later
 * version may append fields; its first four are read.
 */
export function parseTraceparent(
  value: string,
): { traceId: string; spanId: string } | null {
  const match = TRACEPARENT.exec(value.trim())
  if (!match) return null
  const [, version, traceId, spanId, , rest] = match
  if (version === 'ff' || (version === '00' && rest !== undefined)) return null
  if (!traceId || !spanId || /^0+$/.test(traceId) || /^0+$/.test(spanId))
    return null
  return { traceId, spanId }
}

function nonZeroHex(random: (bytes: number) => Uint8Array, bytes: number) {
  for (;;) {
    const hex = Buffer.from(random(bytes)).toString('hex')
    if (/[^0]/.test(hex)) return hex
  }
}

function randomBytes(bytes: number): Uint8Array {
  return randomFillSync(new Uint8Array(bytes))
}
