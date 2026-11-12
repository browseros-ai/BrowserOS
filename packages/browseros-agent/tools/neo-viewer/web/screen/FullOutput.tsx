/**
 * "Open full": one run's reply, or one call's output (`operation-output.ts`), in a large pop-up,
 * with line numbers, find, copy, and previous / next: across the finished runs for a reply,
 * through the run's calls in order for a call. It is a native modal `<dialog>`: the page behind
 * it is inert, focus stays inside, Escape closes it, and focus returns to what opened it.
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import type { NeoOperation, RunDetail, RunSummary } from '../../shared/api'
import { formatCode } from '../../shared/code-format'
import { useRunDetail } from '../use-viewer'
import { findMatches, markLine, rangesByLine, stepMatch } from './find'
import { operationOutput } from './operation-output'
import {
  callPlaces,
  isNeoInternal,
  type OperationSelection,
  placeOperations,
  placeText,
} from './operations'
import { formattedLines, noReplyReason, rawLines, replyParts } from './reply'
import { codeLines, lineText } from './text-lines'
import { LineSpans, REPLY_VIEWS, type ReplyView, Switch } from './ui'
import { clock, count, plural } from './units'

/** The finished runs on either side of `run`, in call order: what Previous and Next open. */
export function neighbours(
  runs: RunSummary[],
  run: RunSummary,
): { previous: RunSummary | null; next: RunSummary | null } {
  const finished = runs
    .filter((candidate) => candidate.status !== 'running')
    .sort((a, b) => a.seq - b.seq)
  return {
    previous: finished.findLast((candidate) => candidate.seq < run.seq) ?? null,
    next: finished.find((candidate) => candidate.seq > run.seq) ?? null,
  }
}

export function FullOutput({
  run,
  runs,
  onRun,
  operation,
  onOperation,
  onClose,
}: {
  run: RunSummary
  runs: RunSummary[]
  onRun: (run: RunSummary) => void
  /** A call of `run` to show instead of its reply. */
  operation: OperationSelection | null
  onOperation: (selection: OperationSelection) => void
  onClose: () => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const search = useRef<HTMLInputElement>(null)
  const body = useRef<HTMLDivElement>(null)
  const [view, setView] = useState<ReplyView>('formatted')
  const [query, setQuery] = useState('')
  const [current, setCurrent] = useState(0)
  const [copied, setCopied] = useState(false)

  const fetched = useRunDetail(run, true)
  const detail = fetched?.id === run.id ? fetched : null
  const call = useMemo(
    () => (operation && detail ? callIn(run, detail.neo, operation) : null),
    [run, detail, operation],
  )
  const lines = useMemo(
    () =>
      !detail
        ? []
        : call
          ? view === 'formatted'
            ? call.output.formatted
            : call.output.raw
          : view === 'formatted'
            ? formattedLines(detail)
            : rawLines(detail),
    [detail, view, call],
  )
  const texts = useMemo(() => lines.map(lineText), [lines])
  const matches = useMemo(() => findMatches(texts, query), [texts, query])
  const active =
    matches.length === 0 ? -1 : Math.min(current, matches.length - 1)
  const marks = useMemo(() => rangesByLine(matches, active), [matches, active])
  const { previous, next } = neighbours(runs, run)

  // The pop-up opens ready to find. React's autoFocus would run before showModal, while the
  // dialog is still closed, so the field is focused once the dialog is open.
  useEffect(() => {
    dialog.current?.showModal()
    search.current?.focus()
  }, [])

  // A new query, view or output starts again at its first match.
  // biome-ignore lint/correctness/useExhaustiveDependencies: these are the triggers, not inputs.
  useEffect(() => setCurrent(0), [query, view, run.id, operation?.operationId])

  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll whenever the current match moves.
  useEffect(() => {
    body.current
      ?.querySelector('[data-mark="current"]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [active, matches])

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1500)
    return () => clearTimeout(timer)
  }, [copied])

  const step = (direction: 1 | -1) =>
    setCurrent(stepMatch(active, matches.length, direction))

  const text = detail?.reply.text ?? null
  const consoleLines = text === null ? 0 : replyParts(text).console.length
  const frame = framing(run, call, view)
  const toPrevious = call
    ? call.previous && (() => call.previous && onOperation(call.previous))
    : previous && (() => onRun(previous))
  const toNext = call
    ? call.next && (() => call.next && onOperation(call.next))
    : next && (() => onRun(next))
  const { title } = frame

  return (
    <dialog
      ref={dialog}
      aria-label={`${title}, run ${run.seq}`}
      onClose={onClose}
      onKeyDown={(event) => {
        if ((event.metaKey || event.ctrlKey) && event.key === 'f') {
          event.preventDefault()
          search.current?.focus()
          search.current?.select()
        }
      }}
      className="m-auto h-[min(760px,calc(100vh-40px))] w-[min(1000px,calc(100vw-40px))] max-w-none flex-col overflow-clip rounded-[10px] bg-white p-0 font-plex text-ink text-xs leading-4 shadow-[0_24px_60px_#0D173047] backdrop:bg-[#0D173052] open:flex"
    >
      <header className="flex h-[54px] shrink-0 items-center gap-3 border-line border-b pr-4 pl-5">
        <h2 className="font-semibold text-base leading-5">{title}</h2>
        <span className="text-[13px] text-ink-2 leading-4">{frame.where}</span>
        <span className="flex-1" />
        <Switch
          label="Output view"
          options={REPLY_VIEWS}
          value={view}
          onChange={setView}
          size="large"
        />
        <label className="flex h-7 w-[200px] shrink-0 items-center gap-1.5 rounded-md border border-control px-2.5 focus-within:border-neo">
          <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
            <circle
              cx="5"
              cy="5"
              r="3.5"
              fill="none"
              stroke="#6B7385"
              strokeWidth="1.3"
            />
            <path d="M7.7 7.7L11 11" stroke="#6B7385" strokeWidth="1.3" />
          </svg>
          <input
            ref={search}
            // Not type="search": there Escape clears the field and the pop-up stays open.
            type="text"
            aria-label="Find in output"
            placeholder="Find in output"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                step(event.shiftKey ? -1 : 1)
              }
            }}
            className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-ink-3"
          />
          {query !== '' && (
            <span
              className="shrink-0 font-code text-[11px] text-ink-3"
              aria-live="polite"
            >
              {matches.length === 0 ? '0' : `${active + 1}/${matches.length}`}
            </span>
          )}
        </label>
        <button
          type="button"
          disabled={lines.length === 0}
          onClick={() =>
            void navigator.clipboard
              .writeText(texts.join('\n'))
              .then(() => setCopied(true))
          }
          className="rounded-md border border-control px-2.5 py-[5px] font-medium hover:bg-panel disabled:opacity-50"
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
        {call ? (
          <>
            <span className="text-ink">
              <span className="font-code text-neo">
                ≈{count(Math.floor(frame.shownText.length / 4))}
              </span>{' '}
              tok
            </span>
            <span>{count(frame.shownText.length)} chars</span>
            <span>{plural(lines.length, 'line')}</span>
          </>
        ) : run.replySize ? (
          <>
            <span className="text-ink">
              <span className="font-code text-neo">
                ≈{count(run.replySize.tokens)}
              </span>{' '}
              tok
            </span>
            <span>{count(run.replySize.chars)} chars</span>
            <span>{plural(lines.length, 'line')}</span>
            <span>
              {consoleLines === 0
                ? 'no console lines'
                : plural(consoleLines, 'console line')}
            </span>
          </>
        ) : (
          <span>{detail ? noReplyReason(detail) : 'Loading…'}</span>
        )}
        <span className="flex-1" />
        {run.neoRunId && (
          <span>
            Neo run <span className="font-code text-ink">{run.neoRunId}</span>
          </span>
        )}
      </div>
      <div ref={body} className="min-h-0 flex-1 overflow-auto px-5 pt-3.5 pb-5">
        <div className="w-max min-w-full font-code text-[12.5px] leading-5">
          {lines.map((line, index) => {
            const ranges = marks.get(index)
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: an output line is its line number.
              <div key={index} className="flex gap-3.5">
                <span className="w-7 shrink-0 select-none text-right text-ink-4">
                  {index + 1}
                </span>
                <span className="whitespace-pre text-ink">
                  <LineSpans
                    line={line}
                    marked={ranges ? markLine(line, ranges) : undefined}
                  />
                </span>
              </div>
            )
          })}
        </div>
      </div>
      <footer className="flex h-[46px] shrink-0 items-center gap-3 border-line border-t px-5">
        <p className="min-w-0 truncate text-ink-2" title={frame.footer}>
          {frame.footer}
        </p>
        <span className="flex-1" />
        <button
          type="button"
          disabled={!toPrevious}
          onClick={() => toPrevious?.()}
          className="shrink-0 rounded-md border border-control px-2.5 py-[5px] font-medium hover:bg-panel disabled:opacity-40"
        >
          Previous {frame.unit}
        </button>
        <button
          type="button"
          disabled={!toNext}
          onClick={() => toNext?.()}
          className="shrink-0 rounded-md border border-control px-2.5 py-[5px] font-medium hover:bg-panel disabled:opacity-40"
        >
          Next {frame.unit}
        </button>
      </footer>
    </dialog>
  )
}

type Call = NonNullable<ReturnType<typeof callIn>>

/** What the pop-up says: a call's name, place, size and source, or the reply's. */
function framing(run: RunSummary, call: Call | null, view: ReplyView) {
  if (!call) {
    return {
      title: 'What the model read',
      where: `run ${run.seq}, ${clock(run.sentAt, true)}`,
      shownText: '',
      footer: 'The exact reply, captured by the viewer.',
      unit: 'output',
    }
  }
  const formatted = view === 'formatted'
  const internal = isNeoInternal(call.operation) ? ['Neo internal'] : []
  return {
    title: call.operation.name,
    where: [`run ${run.seq}`, placeText(call.place), ...internal].join(' · '),
    shownText: formatted ? call.output.formattedText : call.output.rawText,
    footer: `${formatted ? call.output.source : "Neo's record of the result, as JSON."}${call.output.cut ? ' Neo kept only the start of it.' : ''}`,
    unit: 'call',
  }
}

/**
 * The selected call in the run's record, with its output, where it sits, and the calls before
 * and after it in the order they started. Places are worked out as the run's block does.
 */
function callIn(
  run: RunSummary,
  record: RunDetail['neo'],
  selection: OperationSelection,
) {
  if (!record) return null
  const operation = record.operations.find(
    (candidate) => candidate.id === selection.operationId,
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
  const ordered = [...record.operations].sort((a, b) => a.seq - b.seq)
  const at = ordered.indexOf(operation)
  const selectionOf = (candidate: NeoOperation | undefined) =>
    candidate
      ? {
          runId: run.id,
          operationId: candidate.id,
          place: places.get(candidate.id) ?? { line: null, call: 1, calls: 1 },
        }
      : null
  return {
    operation,
    output: operationOutput(operation, record.operations),
    place: places.get(operation.id) ?? selection.place,
    previous: selectionOf(ordered[at - 1]),
    next: selectionOf(ordered[at + 1]),
  }
}
