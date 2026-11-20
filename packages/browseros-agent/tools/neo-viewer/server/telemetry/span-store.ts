/**
 * Neo's spans in memory, filed by the run that made them (`neo.run.id`, or the prefix of
 * `neo.op.id`), never by trace id: a caller may reuse one trace for a whole session, and its runs
 * must not share spans. A run's operations are keyed by `neo.op.id`, so an exporter that retries a
 * batch replaces spans instead of adding them: a run's spans only ever grow.
 *
 * Bounded: past any limit the run changed longest ago is dropped whole, unless a read is waiting
 * on it (`pin`). A flood of spans cannot grow the process; a span over its run's cap is refused.
 */

import type { NeoSpan, OpSpan, RunSpan } from './otlp-json'

/** Everything one run's spans said so far. */
export interface RunSpans {
  neoRunId: string
  run: RunSpan | null
  /** By `neo.op.id`. */
  ops: Map<string, OpSpan>
}

export interface SpanLimits {
  maxRuns: number
  maxSpansPerRun: number
  maxSpans: number
  /** Estimated from attribute text: bounds a flood of large attributes, not just of spans. */
  maxBytes: number
}

export const SPAN_LIMITS: SpanLimits = {
  maxRuns: 500,
  maxSpansPerRun: 2_000,
  maxSpans: 50_000,
  maxBytes: 64 * 1024 * 1024,
}

/** Per span besides its attributes: ids, times, names. */
const SPAN_OVERHEAD_BYTES = 256

interface Entry extends RunSpans {
  bytes: number
}

export class SpanStore {
  /** Map order is the order runs last changed: eviction takes from the front. */
  private readonly runs = new Map<string, Entry>()
  /** A `neo.run`'s parent span id → its run: binds a run the viewer minted the parent for. */
  private readonly byParent = new Map<string, string>()
  private readonly pins = new Map<string, number>()
  private spans = 0
  private bytes = 0
  /** A neo span has been accepted since the viewer started. */
  spansSeen = false

  constructor(private readonly limits: SpanLimits = SPAN_LIMITS) {}

  /** Files spans under their runs; returns the runs that changed and the spans refused. */
  add(spans: NeoSpan[]): { changed: Set<string>; rejected: number } {
    const changed = new Set<string>()
    let rejected = 0
    for (const span of spans) {
      const entry = this.entry(span.neoRunId)
      // A span already filed (an exporter's retry) is replaced, not added.
      const known = span.kind === 'run' ? entry.run : entry.ops.get(span.opId)
      if (known) {
        const old = this.sizeOf(known)
        this.bytes -= old
        entry.bytes -= old
      } else if (
        span.kind === 'op' &&
        // One place stays free for the run's own `neo.run`.
        entry.ops.size + 1 >= this.limits.maxSpansPerRun
      ) {
        rejected++
        continue
      } else this.spans++
      if (span.kind === 'run') {
        entry.run = span
        if (span.parentSpanId)
          this.byParent.set(span.parentSpanId, span.neoRunId)
      } else entry.ops.set(span.opId, span)
      const size = this.sizeOf(span)
      this.bytes += size
      entry.bytes += size
      // Most recently changed last.
      this.runs.delete(span.neoRunId)
      this.runs.set(span.neoRunId, entry)
      changed.add(span.neoRunId)
      this.spansSeen = true
    }
    this.evict()
    return { changed, rejected }
  }

  get(neoRunId: string): RunSpans | null {
    return this.runs.get(neoRunId) ?? null
  }

  /** The run whose `neo.run` span hangs under this parent span id (one the viewer minted). */
  runUnder(parentSpanId: string): string | null {
    return this.byParent.get(parentSpanId) ?? null
  }

  /** Keeps a run through eviction while a read waits for it; call the result to let it go. */
  pin(neoRunId: string): () => void {
    this.pins.set(neoRunId, (this.pins.get(neoRunId) ?? 0) + 1)
    let released = false
    return () => {
      if (released) return
      released = true
      const left = (this.pins.get(neoRunId) ?? 1) - 1
      if (left > 0) this.pins.set(neoRunId, left)
      else this.pins.delete(neoRunId)
    }
  }

  stats(): { runs: number; spans: number; bytes: number } {
    return { runs: this.runs.size, spans: this.spans, bytes: this.bytes }
  }

  private entry(neoRunId: string): Entry {
    const known = this.runs.get(neoRunId)
    if (known) return known
    const entry: Entry = { neoRunId, run: null, ops: new Map(), bytes: 0 }
    this.runs.set(neoRunId, entry)
    return entry
  }

  private sizeOf(span: NeoSpan): number {
    return SPAN_OVERHEAD_BYTES + JSON.stringify(span.attributes).length
  }

  private over(): boolean {
    return (
      this.runs.size > this.limits.maxRuns ||
      this.spans > this.limits.maxSpans ||
      this.bytes > this.limits.maxBytes
    )
  }

  private evict(): void {
    for (const [neoRunId, entry] of this.runs) {
      if (!this.over()) return
      if (this.pins.has(neoRunId)) continue
      this.runs.delete(neoRunId)
      this.spans -= entry.ops.size + (entry.run ? 1 : 0)
      this.bytes -= entry.bytes
      if (entry.run?.parentSpanId) this.byParent.delete(entry.run.parentSpanId)
    }
  }
}
