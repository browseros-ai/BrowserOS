/**
 * A run's trace where the screen shows it beside the run: the run block's "Trace" row in one
 * line (`TraceLine`, as "Model read" gives the reply), and the sidebar's Trace panel
 * (`TracePanel`): the waterfall, the picked span's attributes as Neo emitted them, and whether
 * any span carries site detail. Both read the trace through `useRunTrace`, which shares one read
 * per health change and stops once the trace is complete.
 */

import type { RunSummary, TraceSpan } from '../../shared/api'
import { traceMayGrow, useRunTrace } from '../use-telemetry'
import { useRunDetail } from '../use-viewer'
import type { OperationSelection } from './operations'
import { operationFor, selectedSpan, shortTraceId, siteDetail } from './trace'
import { OpenIcon } from './ui'
import { SpanAttributes, TraceFacts, Waterfall } from './Waterfall'

/** The sidebar shows a run's trace: the span picked in its waterfall, if any. */
export interface TraceSelection {
  runId: string
  spanId: string | null
}

/**
 * The run's trace in one line, as "Model read" gives the reply: its trace id, its spans, what
 * failed and how long it took; clicking it shows the waterfall on the right.
 */
export function TraceLine({
  run,
  rev,
  shown,
  onSelect,
  onOpenFull,
}: {
  run: RunSummary
  rev: number
  shown: boolean
  onSelect: () => void
  onOpenFull: () => void
}) {
  // A running run has no spans yet: its trace is first read when it ends (its status changes).
  const trace = useRunTrace(run, run.status !== 'running', rev)
  if (run.status === 'running')
    return (
      <span className="text-[11px] text-ink-2">
        Spans arrive when the run ends.
      </span>
    )
  if (trace === undefined)
    return <span className="text-[11px] text-ink-3">Loading…</span>
  if (trace === null)
    return (
      <span className="text-[11px] text-ink-2">
        {traceMayGrow(run, trace, Date.now())
          ? "Waiting for Neo's spans."
          : 'No spans from Neo for this run.'}
      </span>
    )
  return (
    <span className="flex min-w-0 flex-1 items-center gap-2.5">
      <button
        type="button"
        onClick={onSelect}
        title={`Trace ${trace.traceId}: show its waterfall in the right panel`}
        className="flex min-w-0 items-center gap-2.5 text-left"
      >
        <code className="shrink-0 rounded border border-neo-edge bg-white px-2 py-0.5 font-code text-ink">
          {shortTraceId(trace.traceId)}
        </code>
        <span className="min-w-0 truncate whitespace-pre text-[11px] text-ink-2 leading-[14px]">
          <TraceFacts trace={trace} />
          {shown && ', shown on the right'}
        </span>
      </button>
      <button
        type="button"
        onClick={onOpenFull}
        className="shrink-0 font-medium text-neo hover:underline"
      >
        Open full
      </button>
    </span>
  )
}

/**
 * A run's trace: its spans as a waterfall, the selected span's attributes as Neo emitted them,
 * and whether any span carries site detail. Picking a span opens its call in the run's block;
 * the call's own output is one link away. Read again as spans arrive, until `neo.run` has.
 */
export function TracePanel({
  run,
  spanId,
  operationId,
  rev,
  onSelectSpan,
  onShowCall,
  onShowReply,
  onOpenFull,
}: {
  run: RunSummary
  spanId: string | null
  operationId: string | null
  rev: number | undefined
  onSelectSpan: (span: TraceSpan, call: OperationSelection | null) => void
  /** Null when no call is selected. */
  onShowCall: (() => void) | null
  onShowReply: () => void
  onOpenFull: () => void
}) {
  const trace = useRunTrace(run, true, rev)
  const fetched = useRunDetail(run, true)
  const detail = fetched?.id === run.id ? fetched : null
  const span = trace ? selectedSpan(trace, spanId, operationId) : null
  const injected = run.trace?.injected ?? false
  return (
    <section
      aria-label="Trace"
      className="flex min-h-[280px] flex-1 flex-col gap-2"
    >
      <div className="flex items-center gap-2">
        <h2 className="font-semibold text-[13px] text-ink leading-4">Trace</h2>
        <span className="text-ink-2 text-xs leading-4">run {run.seq}</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onOpenFull}
          disabled={!trace}
          className="flex items-center gap-[5px] rounded-[5px] border border-control px-2 py-[3px] font-medium text-ink text-xs leading-4 hover:bg-panel disabled:opacity-50"
        >
          <OpenIcon />
          Open full
        </button>
      </div>
      <p
        className="min-w-0 shrink-0 truncate text-[11px] text-ink-2 leading-[14px]"
        title={trace ? `Trace ${trace.traceId}` : undefined}
      >
        {trace ? (
          <>
            <span className="font-code text-ink">
              {shortTraceId(trace.traceId)}
            </span>{' '}
            · <TraceFacts trace={trace} /> ·{' '}
            {injected
              ? 'traceparent from the viewer'
              : run.trace
                ? 'traceparent from the caller'
                : 'no traceparent'}
          </>
        ) : trace === null ? (
          traceMayGrow(run, trace, Date.now()) ? (
            "Waiting for Neo's spans: they arrive about half a second after the run ends."
          ) : (
            'No spans from Neo for this run: an image older than telemetry, or none were sent.'
          )
        ) : (
          'Loading…'
        )}
      </p>
      {trace && (
        <Waterfall
          trace={trace}
          selectedSpanId={span?.spanId ?? null}
          onSelectSpan={(picked) =>
            onSelectSpan(picked, operationFor(run, detail?.neo ?? null, picked))
          }
        />
      )}
      {span && <SpanAttributes span={span} />}
      {trace && (
        <p className="shrink-0 text-[11px] text-ink-2 leading-4">
          {siteDetail(trace).text}
        </p>
      )}
      <p className="mt-auto shrink-0 text-[11px] text-ink-2 leading-4">
        Exactly what Neo emitted, as the viewer received it.{' '}
        {onShowCall && (
          <>
            <button
              type="button"
              onClick={onShowCall}
              className="font-medium text-neo hover:underline"
            >
              Show what the call returned
            </button>{' '}
          </>
        )}
        <button
          type="button"
          onClick={onShowReply}
          className="font-medium text-neo hover:underline"
        >
          Show the reply
        </button>
      </p>
    </section>
  )
}
