/**
 * A run's OpenTelemetry spans as a waterfall: `neo.run` first, its `neo.op` children under it (an
 * op that ran inside another one step further in), each a bar on the run's own time axis with its
 * duration, red when it failed; and the selected span's attributes exactly as Neo emitted them,
 * which is how a developer checks what leaves Neo, redaction included. Drawn in the sidebar's
 * Trace panel and, wide, in its "Open full" pop-up (`TraceFull`); both select through the screen,
 * so the run block's method rows follow (`trace.ts` says how a span names its call).
 *
 * Rows borrow the run block's method lanes: the label, an outcome dot, a bar, its duration; the
 * run's own span is a light track the operations sit on.
 */

import { useEffect, useRef, useState } from 'react'
import type { RunSummary, RunTrace, TraceSpan } from '../../shared/api'
import { attributeLines, siteDetail, spanBar, spanName } from './trace'
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
  return (
    <div className="flex flex-col" data-waterfall="">
      <div className="flex h-5 shrink-0 items-end px-2 pb-0.5">
        <span className={`${name} shrink-0 ${label}`}>Span</span>
        <span className="w-3.5 shrink-0" />
        <span className="flex min-w-0 flex-1 justify-between font-code text-[10px] text-ink-3 leading-[14px]">
          <span>0</span>
          <span>{seconds(trace.durationMs)}</span>
        </span>
        <span className="w-11 shrink-0" />
      </div>
      {trace.spans.map((span) => (
        <SpanRow
          key={span.spanId}
          span={span}
          durationMs={trace.durationMs}
          name={name}
          selected={span.spanId === selectedSpanId}
          onSelect={() => onSelectSpan(span)}
        />
      ))}
    </div>
  )
}

function SpanRow({
  span,
  durationMs,
  name,
  selected,
  onSelect,
}: {
  span: TraceSpan
  durationMs: number
  name: string
  selected: boolean
  onSelect: () => void
}) {
  const bar = spanBar(span, durationMs)
  const run = span.name === 'neo.run'
  // The run's own span is the track its operations sit on: light, and only tinted when picked.
  const tone = span.error
    ? 'bg-bad'
    : run
      ? selected
        ? 'bg-neo-soft'
        : 'bg-control'
      : selected
        ? 'bg-neo'
        : 'bg-ink-2'
  return (
    <button
      type="button"
      aria-pressed={selected}
      aria-label={`${spanName(span)}, ${seconds(span.durationMs)}${span.error ? ', failed' : ''}`}
      data-span={span.spanId}
      onClick={onSelect}
      className={`flex h-6 w-full shrink-0 items-center px-2 text-left font-code text-[11px] leading-[14px] ${selected ? 'bg-[#DCE6FB]' : 'hover:bg-neo-tint'}`}
    >
      <span
        className={`${name} shrink-0 truncate pr-2 ${run ? 'font-semibold text-black' : selected ? 'text-black' : 'text-ink'}`}
        style={{ paddingLeft: `${Math.max(0, span.depth) * 10}px` }}
        title={`${span.name} ${span.label}`}
      >
        {spanName(span)}
      </span>
      <span className="flex w-3.5 shrink-0 justify-center">
        <span
          className={`block size-1.5 rounded-full ${span.error ? 'bg-bad' : 'bg-ok'}`}
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
        className={`w-11 shrink-0 text-right ${selected ? 'font-semibold text-black' : 'text-ink'}`}
      >
        {seconds(span.durationMs)}
      </span>
    </button>
  )
}

/** `traceFacts`, with what failed in the error tone. */
export function TraceFacts({ trace }: { trace: RunTrace }) {
  const failed = trace.spans.filter((span) => span.error).length
  return (
    <>
      {plural(trace.spans.length, 'span')}
      {failed > 0 && (
        <>
          , <span className="text-bad">{failed} failed</span>
        </>
      )}
      , {seconds(trace.durationMs)}
      {!trace.complete && (
        <span className="text-busy-ink">, waiting for neo.run</span>
      )}
    </>
  )
}

/** The selected span: what it is, then every attribute Neo put on it, as sent. */
export function SpanAttributes({ span }: { span: TraceSpan }) {
  const lines = attributeLines(span.attributes)
  return (
    <div className="flex min-w-0 flex-col gap-1.5" data-attributes="">
      <div className="flex min-w-0 items-baseline gap-2">
        <h3 className="min-w-0 truncate font-semibold text-ink text-xs leading-4">
          {spanName(span)}
        </h3>
        <span className="shrink-0 text-[11px] text-ink-2 leading-[14px]">
          {span.name}, span <span className="font-code">{span.spanId}</span>
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
  useEffect(() => {
    dialog.current?.showModal()
  }, [])
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1500)
    return () => clearTimeout(timer)
  }, [copied])
  const at = trace.spans.findIndex((span) => span.spanId === selectedSpanId)
  const span = trace.spans[at] ?? trace.spans[0] ?? null
  const index = span ? trace.spans.indexOf(span) : -1
  const previous = index > 0 ? trace.spans[index - 1] : undefined
  const next = index >= 0 ? trace.spans[index + 1] : undefined
  const resource = Object.entries(trace.resource)
  return (
    <dialog
      ref={dialog}
      aria-label={`Trace, run ${run.seq}`}
      onClose={onClose}
      className="m-auto h-[min(760px,calc(100vh-40px))] w-[min(1000px,calc(100vw-40px))] max-w-none flex-col overflow-clip rounded-[10px] bg-white p-0 font-plex text-ink text-xs leading-4 shadow-[0_24px_60px_#0D173047] backdrop:bg-[#0D173052] open:flex"
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
              .then(() => setCopied(true))
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
          Exactly what Neo emitted, as the viewer received it over OTLP.
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
