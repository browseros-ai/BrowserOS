/**
 * The Traces tab browses actual received OTLP run slices, then opens one slice's existing
 * waterfall. Trace IDs can be shared by runs, so selection and navigation use run ID + send
 * time. Metrics supplies one method/duration filter; the local trace-ID search composes with it.
 */
import { useEffect, useState } from 'react'
import {
  apiRoutes,
  type RunDetail,
  type RunResponse,
  type RunSummary,
  type TraceSearchResult,
  type TraceSpan,
} from '../../shared/api'
import { useRunTraceRead } from '../use-telemetry'
import { useTraceSearch } from '../use-trace-search'
import { getJson } from '../use-viewer'
import type { ExplorerSpanFilter } from './explorer-filter'
import type { OperationSelection } from './operations'
import { operationFor, selectedSpan, shortTraceId, spanEnd } from './trace'
import { clock, plural, seconds } from './units'
import { SpanAttributes, TraceFacts, Waterfall } from './Waterfall'

interface BoundTrace {
  result: TraceSearchResult
  run: RunSummary
  key: string
}

export function TracesExplorer({
  runs,
  rev,
  filter,
  onClearFilter,
  onOpenRun,
}: {
  runs: RunSummary[]
  rev: number | undefined
  filter: ExplorerSpanFilter | null
  onClearFilter: () => void
  onOpenRun: (
    run: RunSummary,
    span: TraceSpan | null,
    operation?: OperationSelection | null,
  ) => void
}) {
  const [traceId, setTraceId] = useState('')
  const [picked, setPicked] = useState<string | null>(null)
  const search = useTraceSearch(runs, rev, traceId, filter)
  const byRun = new Map(runs.map((run) => [run.id, run]))
  // A Clear or a removed run can overtake the search response. Never display an old run or
  // bind its spans to a newly created run with the same ID.
  const rows: BoundTrace[] =
    search.kind === 'ready'
      ? search.answer.traces.flatMap((result) => {
          const run = byRun.get(result.run.id)
          return run?.sentAt === result.run.sentAt
            ? [{ result, run, key: `${run.id}@${run.sentAt}` }]
            : []
        })
      : []
  const selected = rows.find((row) => row.key === picked) ?? rows[0]
  const selectedKey = selected?.key ?? null
  // Anchor the automatic first selection too. New runs sort above it and must not replace
  // the waterfall someone is inspecting while telemetry refreshes.
  useEffect(() => setPicked(selectedKey), [selectedKey])
  const filtered = traceId.length > 0 || filter !== null
  const range = filter
    ? `>${seconds(filter.minMs)}${filter.maxMs === null ? '' : ` to ${seconds(filter.maxMs)}`}`
    : ''
  return (
    <section
      aria-label="Traces explorer"
      className="mx-auto flex w-full max-w-[1536px] flex-col gap-4 px-6 pt-6 pb-8 lg:px-[46px]"
    >
      <header className="flex items-center gap-4">
        <div>
          <h1 className="font-semibold text-[30px] text-ink leading-10">
            Traces
          </h1>
          <p className="text-[15px] text-ink-2 leading-5">
            Inspect the OpenTelemetry spans received from Neo.
          </p>
        </div>
        <span className="flex-1" />
        <p className="flex shrink-0 items-center gap-2 text-ink-2 text-xs">
          <span className="size-1.5 rounded-full bg-ok" />
          Recorded in this viewer
        </p>
      </header>
      <div className="flex flex-wrap items-center gap-2">
        <input
          aria-label="Find trace by ID"
          type="search"
          value={traceId}
          onChange={(event) => setTraceId(event.target.value)}
          placeholder="Find trace by ID…"
          spellCheck={false}
          className="h-9 w-[285px] rounded-[5px] border border-control bg-white px-3 text-ink text-sm outline-none placeholder:text-ink-2 focus:border-neo"
        />
        {filter && (
          <>
            <span className="rounded bg-neo-tint px-2.5 py-1 text-neo text-xs">
              Method <span className="font-code">{filter.method}</span>
            </span>
            <span className="rounded bg-neo-tint px-2.5 py-1 text-neo text-xs">
              Duration {range}
            </span>
          </>
        )}
        <span className="flex-1" />
        {filtered && (
          <button
            type="button"
            onClick={() => {
              setTraceId('')
              onClearFilter()
            }}
            className="font-semibold text-neo text-sm hover:underline"
          >
            Clear filters
          </button>
        )}
      </div>
      <div>
        <div className="flex min-h-9 items-center gap-3 border-line border-b pb-2">
          <h2 aria-live="polite" className="font-semibold text-[15px]">
            {search.kind === 'loading'
              ? 'Finding traces…'
              : `${plural(rows.length, filtered ? 'matching trace' : 'trace')}${search.kind === 'ready' && search.answer.truncated ? ' shown' : ''}`}
          </h2>
          {search.kind === 'ready' && search.answer.truncated && (
            <button
              type="button"
              onClick={search.loadMore}
              disabled={search.loadingMore}
              className="font-semibold text-neo text-xs hover:underline disabled:text-ink-2"
            >
              {search.loadingMore ? 'Loading more…' : 'Load more traces'}
            </button>
          )}
        </div>
        {(search.kind === 'failed' ||
          (search.kind === 'ready' && search.failed)) && (
          <p role="status" className="py-3 text-bad text-sm">
            Could not refresh traces. Retrying when telemetry updates.
          </p>
        )}
        <div className="overflow-x-auto">
          <div className="min-w-[830px]">
            <div
              className={`${ROW_GRID} h-8 text-[10px] text-ink-2 uppercase tracking-wide`}
            >
              <span>Trace ID</span>
              <span>Started</span>
              <span>Run</span>
              <span>Matching operation</span>
              <span>Span time</span>
              <span>Trace time</span>
              <span>Spans</span>
            </div>
            {rows.map((row) => (
              <button
                key={row.key}
                type="button"
                data-trace-run={row.run.id}
                aria-pressed={row.key === selected?.key}
                onClick={() => setPicked(row.key)}
                className={`${ROW_GRID} min-h-[46px] w-full border-l-[3px] text-left text-xs ${row.key === selected?.key ? 'border-neo bg-neo-tint' : 'border-transparent hover:bg-panel'}`}
              >
                <span
                  className="truncate font-code font-semibold text-neo"
                  title={row.result.traceId}
                >
                  {shortTraceId(row.result.traceId)}…
                </span>
                <span className="font-code">{clock(row.result.startedAt)}</span>
                <span>Run {row.run.seq}</span>
                <span
                  className="truncate font-code"
                  title={row.result.matchedSpan?.label}
                >
                  {row.result.matchedSpan?.label ?? 'All spans'}
                </span>
                <span className="font-code font-semibold">
                  {row.result.matchedSpan
                    ? seconds(row.result.matchedSpan.durationMs)
                    : '·'}
                </span>
                <span className="font-code">
                  {seconds(row.result.durationMs)}
                </span>
                <span className="font-code">{row.result.spanCount}</span>
              </button>
            ))}
          </div>
        </div>
        {search.kind === 'ready' && rows.length === 0 && (
          <p className="py-8 text-ink-2 text-sm">
            {filtered
              ? 'No received spans match these filters.'
              : 'No traces received for the recorded runs yet.'}
          </p>
        )}
      </div>
      {selected && (
        <TraceDetail
          key={`${selected.key}:${filter?.method ?? ''}:${filter?.minMs ?? ''}:${filter?.maxMs ?? ''}`}
          row={selected}
          rev={rev}
          onOpenRun={onOpenRun}
        />
      )}
    </section>
  )
}

const ROW_GRID =
  'grid grid-cols-[minmax(140px,1.15fr)_120px_95px_minmax(180px,1.4fr)_100px_100px_50px] items-center gap-3 px-3'

/** Owns selection for one bound run, so late reads cannot move another run's selected span. */
function TraceDetail({
  row: { result, run },
  rev,
  onOpenRun,
}: {
  row: BoundTrace
  rev: number | undefined
  onOpenRun: (
    run: RunSummary,
    span: TraceSpan | null,
    operation?: OperationSelection | null,
  ) => void
}) {
  const traceRead = useRunTraceRead(run, true, rev, 'live')
  const trace = traceRead.trace
  const request = `${run.id}@${run.sentAt}#${run.status}#${run.operationCount}#${run.neoDurationMs}`
  const [detailRead, setDetailRead] = useState<{
    request: string
    detail: RunDetail | null
  } | null>(null)
  // Unlike the usual read-only panels, navigation must distinguish an unfinished read from
  // a failed one. Wait for the former; a missing/older Neo record still permits opening the run.
  // biome-ignore lint/correctness/useExhaustiveDependencies: request includes every summary field that changes the run's record.
  useEffect(() => {
    let current = true
    getJson<RunResponse>(apiRoutes.run(run.id)).then(
      (answer) => {
        if (current) setDetailRead({ request, detail: answer.run })
      },
      () => {
        if (current) setDetailRead({ request, detail: null })
      },
    )
    return () => {
      current = false
    }
  }, [request])
  const fetched = detailRead?.detail
  const detail =
    fetched?.id === run.id && fetched.sentAt === run.sentAt ? fetched : null
  const [spanId, setSpanId] = useState<string | null>(
    result.matchedSpan?.spanId ?? null,
  )
  const [copied, setCopied] = useState(false)
  const span = trace ? selectedSpan(trace, spanId, null) : null
  // Anchor the first automatic pick too. New spans and retried exports must not move it.
  useEffect(() => {
    if (spanId === null && span) setSpanId(span.spanId)
  }, [spanId, span])
  const call = span ? operationFor(run, detail?.neo ?? null, span) : null
  const opening =
    traceRead.kind === 'loading' || detailRead?.request !== request
  let openHelp: string | undefined
  if (opening) openHelp = 'Waiting for the span and run record.'
  else if (!span) openHelp = 'Open the run; its spans could not be loaded.'
  else if (span.opId && !call)
    openHelp =
      'Open the run and span; this operation is not in the available run record.'
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1500)
    return () => clearTimeout(timer)
  }, [copied])
  return (
    <section
      aria-label={`Trace for run ${run.seq}`}
      className="flex flex-col gap-4 pt-2"
    >
      <div>
        <div className="flex flex-wrap items-center gap-2.5">
          <h2 className="font-semibold text-[22px] leading-8">
            Trace {shortTraceId(result.traceId)}
          </h2>
          <span
            className={`rounded px-2 py-1 text-xs ${run.status === 'ok' ? 'bg-[#EAF6EF] text-ok' : run.status === 'running' ? 'bg-busy-tint text-busy-ink' : 'bg-[#FCEEF0] text-bad'}`}
          >
            Run {run.status === 'ok' ? 'succeeded' : run.status}
          </span>
          {result.errorCount > 0 && (
            <span className="rounded bg-[#FCEEF0] px-2 py-1 text-bad text-xs">
              {plural(result.errorCount, 'failed span')}
            </span>
          )}
          <span className="flex-1" />
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard?.writeText(result.traceId).then(
                () => setCopied(true),
                () => {},
              )
            }}
            className="rounded-md border border-control px-3 py-2 font-semibold text-xs hover:bg-panel"
          >
            {copied ? 'Copied' : 'Copy trace ID'}
          </button>
          <button
            type="button"
            disabled={opening}
            title={openHelp}
            onClick={() => onOpenRun(run, span, call)}
            className="rounded-md border border-control px-3 py-2 font-semibold text-xs hover:bg-panel disabled:opacity-50"
          >
            Open in Runs ↗
          </button>
        </div>
        <p className="mt-1 text-ink-2 text-xs" title={result.traceId}>
          {trace ? (
            <TraceFacts trace={trace} />
          ) : (
            `${seconds(result.durationMs)} · ${plural(result.spanCount, 'span')}`
          )}
          {' · '}Run {run.seq}
          {trace?.resource['service.name']
            ? ` · ${trace.resource['service.name']}`
            : ''}
        </p>
      </div>
      <div className="flex flex-col items-start gap-5 xl:flex-row">
        <div className="w-full min-w-0 flex-1 overflow-hidden rounded-[7px] border border-line">
          <h3 className="border-line border-b px-4 py-3 font-semibold text-base">
            Span waterfall
          </h3>
          {trace ? (
            <div className="overflow-x-auto py-2">
              <div className="min-w-[550px] [&_[data-span]]:h-9">
                <Waterfall
                  trace={trace}
                  selectedSpanId={span?.spanId ?? null}
                  onSelectSpan={(picked) => setSpanId(picked.spanId)}
                />
              </div>
            </div>
          ) : (
            <p className="p-4 text-ink-2 text-sm">
              {trace === undefined
                ? traceRead.kind === 'failed'
                  ? 'Could not load spans.'
                  : 'Loading spans…'
                : 'These spans are no longer available.'}
            </p>
          )}
        </div>
        <aside className="flex w-full min-w-0 flex-col gap-3 overflow-hidden rounded-[7px] border border-line p-4 xl:w-[326px] xl:shrink-0">
          <h3 className="font-semibold text-[15px]">Selected span</h3>
          {span ? (
            <>
              <p
                className={`text-xs ${spanEnd(span) === 'failed' ? 'text-bad' : 'text-ink-2'}`}
              >
                {spanEnd(span)} · {seconds(span.durationMs)}
              </p>
              <SpanAttributes span={span} />
            </>
          ) : (
            <p className="text-ink-2 text-xs">
              Select a received span to see its attributes.
            </p>
          )}
        </aside>
      </div>
    </section>
  )
}
