/**
 * A run's OpenTelemetry spans as a waterfall: `neo.run` first, its operations' spans under it (an
 * op that ran inside another one step further in), each a bar on the run's own time axis with its
 * duration, red when it failed; and the selected span's attributes exactly as Neo emitted them,
 * which is how a developer checks what leaves Neo, redaction included. Drawn in the sidebar's
 * Trace panel and, wide, in its "Open full" pop-up (`TraceFull`); both select through the screen,
 * so the run block's method rows follow (`trace.ts` says how a span names its call).
 *
 * Rows borrow the run block's method lanes: the label, an outcome dot, a bar, its duration; the
 * run's own span is a light track the operations sit on. The script's own operations come first;
 * Neo's own work for the run (operations with no source line, `isInternalSpan`: per open tab
 * `page.frames` and `events.listen`, the `events.next` long-poll) folds under one quiet row, as the
 * run block folds it, and opens when clicked or when one of its spans is picked. An operation is
 * red when Neo says it failed (`neo.op.outcome`, never the span's status): a cancelled op is not.
 */

import { useEffect, useRef, useState } from 'react'
import type { RunSummary, RunTrace, TraceSpan } from '../../shared/api'
import {
  attributeLines,
  namesWithCounts,
  siteDetail,
  spanBar,
  spanEnd,
  spanName,
  splitSpans,
  type TraceFact,
  traceFacts,
} from './trace'
import { LineSpans } from './ui'
import { clock, plural, seconds } from './units'

export function Waterfall({
  trace,
  selectedSpanId,
  onSelectSpan,
  wide = false,
}: {
  trace: RunTrace
  selectedSpanId: string | null
  onSelectSpan: (span: TraceSpan) => void
  /** The pop-up's width: a wider label column. */
  wide?: boolean
}) {
  const name = wide ? 'w-[280px]' : 'w-[150px]'
  const label = 'text-[10px] text-ink-3 uppercase leading-[14px] tracking-wide'
  const { script, internal } = splitSpans(trace.spans)
  const [unfolded, setUnfolded] = useState(false)
  // A picked span of Neo's own work (from the run block's "Neo internal" rows) shows itself.
  const open =
    unfolded || internal.some((span) => span.spanId === selectedSpanId)
  const row = (span: TraceSpan, quiet: boolean) => (
    <SpanRow
      key={span.spanId}
      span={span}
      durationMs={trace.durationMs}
      name={name}
      quiet={quiet}
      selected={span.spanId === selectedSpanId}
      onSelect={() => onSelectSpan(span)}
    />
  )
  return (
    <div className="flex shrink-0 flex-col" data-waterfall="">
      <div className="flex h-5 shrink-0 items-end px-2 pb-0.5">
        <span className={`${name} shrink-0 ${label}`}>Span</span>
        <span className="w-3.5 shrink-0" />
        <span className="flex min-w-0 flex-1 justify-between font-code text-[10px] text-ink-3 leading-[14px]">
          <span>0</span>
          <span>{seconds(trace.durationMs)}</span>
        </span>
        <span className="w-11 shrink-0" />
      </div>
      {script.map((span) => row(span, false))}
      {internal.length > 0 && (
        <button
          type="button"
          aria-expanded={open}
          data-internal={internal.length}
          onClick={() => setUnfolded(!open)}
          title={`Neo's own work for the run, not a line of the script: ${namesWithCounts(internal)}`}
          className="flex h-6 w-full shrink-0 items-center px-2 text-left text-[11px] text-ink-3 leading-[14px] hover:bg-neo-tint"
        >
          <span className="w-2.5 shrink-0">{open ? '▾' : '▸'}</span>
          <span className="min-w-0 truncate">
            Neo internal: {plural(internal.length, 'operation')} ·{' '}
            {namesWithCounts(internal)}
          </span>
        </button>
      )}
      {open && internal.map((span) => row(span, true))}
    </div>
  )
}

/** The order rows are drawn and stepped through: the script's own, then Neo's own work. */
export function drawOrder(trace: RunTrace): TraceSpan[] {
  const { script, internal } = splitSpans(trace.spans)
  return [...script, ...internal]
}

const OUTCOME_DOT = { ok: 'bg-ok', failed: 'bg-bad', cancelled: 'bg-ink-4' }

function SpanRow({
  span,
  durationMs,
  name,
  quiet,
  selected,
  onSelect,
}: {
  span: TraceSpan
  durationMs: number
  name: string
  /** Neo's own work, drawn quieter than the script's calls. */
  quiet: boolean
  selected: boolean
  onSelect: () => void
}) {
  const bar = spanBar(span, durationMs)
  const run = span.type === 'run'
  const end = spanEnd(span)
  // The run's own span is the track its operations sit on: light, only tinted when picked, and
  // never red (its status shows on its dot); an operation is red when Neo says it failed.
  const tone = run
    ? selected
      ? 'bg-neo-soft'
      : 'bg-control'
    : end === 'failed'
      ? 'bg-bad'
      : selected
        ? 'bg-neo'
        : quiet
          ? 'bg-ink-4'
          : 'bg-ink-2'
  return (
    <button
      type="button"
      aria-pressed={selected}
      aria-label={`${spanName(span)}, ${seconds(span.durationMs)}${end === 'ok' ? '' : `, ${end}`}`}
      data-span={span.spanId}
      onClick={onSelect}
      className={`flex h-6 w-full shrink-0 items-center px-2 text-left font-code text-[11px] leading-[14px] ${selected ? 'bg-[#DCE6FB]' : 'hover:bg-neo-tint'}`}
    >
      <span
        className={`${name} shrink-0 truncate pr-2 ${run ? 'font-semibold text-black' : selected ? 'text-black' : quiet ? 'text-ink-3' : 'text-ink'}`}
        style={{ paddingLeft: `${Math.max(0, span.depth) * 10}px` }}
        title={
          span.name === span.label ? span.name : `${span.name} ${span.label}`
        }
      >
        {spanName(span)}
      </span>
      <span className="flex w-3.5 shrink-0 justify-center">
        <span
          title={end}
          className={`block size-1.5 rounded-full ${OUTCOME_DOT[end]}`}
        />
      </span>
      <span className="relative h-2 min-w-0 flex-1">
        <span
          data-bar="span"
          className={`absolute inset-y-0 block ${tone}`}
          style={{
            left: `min(${bar.left * 100}%, calc(100% - 2px))`,
            width: `max(2px, ${bar.width * 100}%)`,
          }}
        />
      </span>
      <span
        className={`w-11 shrink-0 text-right ${selected ? 'font-semibold text-black' : quiet ? 'text-ink-3' : 'text-ink'}`}
      >
        {seconds(span.durationMs)}
      </span>
    </button>
  )
}

const FACT_TONE: Record<TraceFact['tone'], string | undefined> = {
  plain: undefined,
  bad: 'text-bad',
  busy: 'text-busy-ink',
}

/** A trace's facts (`traceFacts`) joined by commas, each in its tone. */
export function TraceFacts({ trace }: { trace: RunTrace }) {
  return (
    <>
      {traceFacts(trace).map((fact, index) => (
        <span key={fact.text} className={FACT_TONE[fact.tone]}>
          {index > 0 && ', '}
          {fact.text}
        </span>
      ))}
    </>
  )
}

/** The selected span: what it is, then every attribute Neo put on it, as sent. */
export function SpanAttributes({ span }: { span: TraceSpan }) {
  const lines = attributeLines(span.attributes)
  return (
    <div className="flex min-w-0 shrink-0 flex-col gap-1.5" data-attributes="">
      <div className="flex min-w-0 items-baseline gap-2">
        <h3 className="min-w-0 truncate font-semibold text-ink text-xs leading-4">
          {spanName(span)}
        </h3>
        <span className="shrink-0 text-[11px] text-ink-2 leading-[14px]">
          {span.name === spanName(span) ? '' : `${span.name}, `}span{' '}
          <span className="font-code">{span.spanId}</span>
        </span>
      </div>
      <div className="overflow-auto rounded-[5px] bg-panel">
        <pre className="w-max min-w-full px-3 py-2.5 font-code text-[11px] text-ink leading-[15px]">
          {lines.length === 0 ? (
            <span className="text-ink-3">No attributes.</span>
          ) : (
            lines.map((line) => (
              <div key={line[0]?.text}>
                <LineSpans line={line} />
              </div>
            ))
          )}
        </pre>
      </div>
    </div>
  )
}

/**
 * "Open full" for a trace: the waterfall wide, the selected span's attributes beside it, the
 * resource Neo sent once, and previous / next span. A native modal `<dialog>`, as `FullOutput`:
 * Escape closes it and focus returns to what opened it.
 */
export function TraceFull({
  run,
  trace,
  selectedSpanId,
  onSelectSpan,
  onClose,
}: {
  run: RunSummary
  trace: RunTrace
  selectedSpanId: string | null
  onSelectSpan: (span: TraceSpan) => void
  onClose: () => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [copied, setCopied] = useState(false)
  // The pop-up takes focus itself, not its first button, so ↑ and ↓ step through the spans at
  // once and no button opens looking pressed.
  useEffect(() => {
    dialog.current?.showModal()
    dialog.current?.focus()
  }, [])
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1500)
    return () => clearTimeout(timer)
  }, [copied])
  // Previous and next follow the rows as drawn: the script's own, then Neo's own work.
  const order = drawOrder(trace)
  const at = order.findIndex((span) => span.spanId === selectedSpanId)
  const span = order[at] ?? order[0] ?? null
  const index = span ? order.indexOf(span) : -1
  const previous = index > 0 ? order[index - 1] : undefined
  const next = index >= 0 ? order[index + 1] : undefined
  const resource = Object.entries(trace.resource)
  return (
    <dialog
      ref={dialog}
      tabIndex={-1}
      aria-label={`Trace, run ${run.seq}`}
      onClose={onClose}
      onKeyDown={(event) => {
        const to =
          event.key === 'ArrowDown'
            ? next
            : event.key === 'ArrowUp'
              ? previous
              : undefined
        if (!to) return
        event.preventDefault()
        onSelectSpan(to)
      }}
      className="m-auto h-[min(760px,calc(100vh-40px))] w-[min(1000px,calc(100vw-40px))] max-w-none flex-col overflow-clip rounded-[10px] bg-white p-0 font-plex text-ink text-xs leading-4 shadow-[0_24px_60px_#0D173047] outline-none backdrop:bg-[#0D173052] open:flex"
    >
      <header className="flex h-[54px] shrink-0 items-center gap-3 border-line border-b pr-4 pl-5">
        <h2 className="font-semibold text-base leading-5">Trace</h2>
        <span className="text-[13px] text-ink-2 leading-4">
          run {run.seq}, {clock(run.sentAt, true)}
        </span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={() =>
            void navigator.clipboard
              .writeText(JSON.stringify(trace, null, 2))
              .then(
                () => setCopied(true),
                // Refused (no permission, page not focused): the button stays as it was.
                () => {},
              )
          }
          title="Copy the trace as the viewer's API serves it, as JSON"
          className="rounded-md border border-control px-2.5 py-[5px] font-medium hover:bg-panel"
        >
          {copied ? 'Copied' : 'Copy'}
        </button>
        <button
          type="button"
          aria-label="Close"
          onClick={() => dialog.current?.close()}
          className="flex size-7 items-center justify-center rounded-md hover:bg-panel"
        >
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
            <path d="M3 3l8 8M11 3l-8 8" stroke="#4A5365" strokeWidth="1.5" />
          </svg>
        </button>
      </header>
      <div className="flex h-9 shrink-0 items-center gap-[18px] border-line-2 border-b bg-panel-2 px-5 text-ink-2">
        <span className="text-ink">
          <TraceFacts trace={trace} />
        </span>
        <span className="min-w-0 truncate">{siteDetail(trace).text}</span>
        <span className="flex-1" />
        <span className="shrink-0">
          trace <span className="font-code text-ink">{trace.traceId}</span>
        </span>
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1 overflow-auto px-3 pt-2 pb-4">
          <Waterfall
            trace={trace}
            selectedSpanId={span?.spanId ?? null}
            onSelectSpan={onSelectSpan}
            wide
          />
        </div>
        <aside className="flex w-[340px] shrink-0 flex-col gap-3 overflow-auto border-line border-l px-5 pt-3.5 pb-4">
          {span && <SpanAttributes span={span} />}
          {resource.length > 0 && (
            <div className="flex flex-col gap-1.5">
              <h3 className="font-semibold text-ink text-xs leading-4">
                Resource
              </h3>
              <div className="overflow-auto rounded-[5px] bg-panel">
                <pre className="w-max min-w-full px-3 py-2.5 font-code text-[11px] text-ink leading-[15px]">
                  {attributeLines(trace.resource).map((line) => (
                    <div key={line[0]?.text}>
                      <LineSpans line={line} />
                    </div>
                  ))}
                </pre>
              </div>
            </div>
          )}
        </aside>
      </div>
      <footer className="flex h-[46px] shrink-0 items-center gap-3 border-line border-t px-5">
        <p className="min-w-0 truncate text-ink-2">
          Exactly what Neo emitted, as the viewer received it over OTLP. ↑ and ↓
          step through the spans.
        </p>
        <span className="flex-1" />
        <button
          type="button"
          disabled={!previous}
          onClick={() => previous && onSelectSpan(previous)}
          className="shrink-0 rounded-md border border-control px-2.5 py-[5px] font-medium hover:bg-panel disabled:opacity-40"
        >
          Previous span
        </button>
        <button
          type="button"
          disabled={!next}
          onClick={() => next && onSelectSpan(next)}
          className="shrink-0 rounded-md border border-control px-2.5 py-[5px] font-medium hover:bg-panel disabled:opacity-40"
        >
          Next span
        </button>
      </footer>
    </dialog>
  )
}
