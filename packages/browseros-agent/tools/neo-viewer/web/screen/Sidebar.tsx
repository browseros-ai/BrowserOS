/**
 * The right sidebar. On top, the live screen docked (PR C2's `LiveScreen`: Follow agent / Control,
 * Pop out, the screen); while it is popped out the dock says where it went. Under it, the
 * selection's panel: a call selected in a run's block shows what that call returned
 * (`OperationPanel`, from `operation-output.ts`); otherwise "What the model read", the selected
 * run's full reply exactly as the proxy captured it. Both are formatted or raw, with their size.
 *
 * The screen is as wide as the sidebar and as tall as Neo's screen ratio makes it, up to the
 * sidebar's visible height, so a wider sidebar is a bigger picture. The reply keeps a readable
 * height below it and the sidebar scrolls.
 */

import { type RefObject, useLayoutEffect, useRef, useState } from 'react'
import type { RunSummary } from '../../shared/api'
import { LiveScreen } from '../live-screen/LiveScreen'
import type { LiveScreenController } from '../live-screen/use-live-screen'
import { useRunDetail } from '../use-viewer'
import { operationOutput } from './operation-output'
import { isNeoInternal, type OperationSelection, placeText } from './operations'
import { formattedLines, noReplyReason, rawLines, replyFacts } from './reply'
import type { Line } from './text-lines'
import { LineSpans, OpenIcon, REPLY_VIEWS, type ReplyView, Switch } from './ui'
import { count, plural } from './units'

export function Sidebar({
  width,
  run,
  operation,
  onShowReply,
  onOpenFull,
  onOpenFullOperation,
  live,
  popped,
  onPopOut,
}: {
  width: number
  run: RunSummary | null
  /** The selected call, when it is one of `run`'s; null shows the reply. */
  operation: OperationSelection | null
  onShowReply: () => void
  onOpenFull: (run: RunSummary) => void
  onOpenFullOperation: (selection: OperationSelection) => void
  live: LiveScreenController
  /** The screen is in the pop-up; the docked panel stays empty so only one view connects. */
  popped: boolean
  onPopOut: () => void
}) {
  const aside = useRef<HTMLElement>(null)
  const viewHeight = useContentHeight(aside)
  return (
    <aside
      ref={aside}
      aria-label="Sidebar"
      style={{ width }}
      className="flex min-h-0 shrink-0 flex-col gap-3 overflow-y-auto px-4 py-3.5"
    >
      <LiveScreen
        live={live}
        size="docked"
        popped={popped}
        onPopOut={onPopOut}
        viewHeight={viewHeight ?? undefined}
      />
      {run && operation ? (
        <OperationPanel
          key={operation.operationId}
          run={run}
          selection={operation}
          onShowReply={onShowReply}
          onOpenFull={() => onOpenFullOperation(operation)}
        />
      ) : run ? (
        <ReplyPanel key={run.id} run={run} onOpenFull={() => onOpenFull(run)} />
      ) : (
        <section className="flex min-h-0 flex-1 flex-col gap-2">
          <h2 className="font-semibold text-[13px] text-ink leading-4">
            What the model read
          </h2>
          <p className="text-ink-2 text-xs leading-4">
            Open a run to read the reply it returned.
          </p>
        </section>
      )}
    </aside>
  )
}

function ReplyPanel({
  run,
  onOpenFull,
}: {
  run: RunSummary
  onOpenFull: () => void
}) {
  const [view, setView] = useState<ReplyView>('formatted')
  const fetched = useRunDetail(run, true)
  const detail = fetched?.id === run.id ? fetched : null
  const text = detail?.reply.text ?? null
  const lines = detail
    ? view === 'formatted'
      ? formattedLines(detail)
      : rawLines(detail)
    : []
  return (
    <section
      aria-label="What the model read"
      className="flex min-h-[280px] flex-1 flex-col gap-2"
    >
      <div className="flex items-center gap-2">
        <h2 className="font-semibold text-[13px] text-ink leading-4">
          What the model read
        </h2>
        <span className="text-ink-2 text-xs leading-4">run {run.seq}</span>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onOpenFull}
          disabled={run.status === 'running'}
          className="flex items-center gap-[5px] rounded-[5px] border border-control px-2 py-[3px] font-medium text-ink text-xs leading-4 hover:bg-panel disabled:opacity-50"
        >
          <OpenIcon />
          Open full
        </button>
      </div>
      <div className="flex items-center gap-2.5">
        <Switch
          label="Reply view"
          options={REPLY_VIEWS}
          value={view}
          onChange={setView}
          size="small"
        />
        <span className="min-w-0 truncate text-[11px] text-ink-2 leading-[14px]">
          {text !== null && run.replySize
            ? replyFacts(text, run.replySize)
            : detail
              ? noReplyReason(detail)
              : run.status === 'running'
                ? "Waiting for Neo's reply."
                : 'Loading…'}
        </span>
      </div>
      <OutputLines lines={lines} />
      <p className="shrink-0 text-[11px] text-ink-2 leading-4">
        Exactly the text the agent got back from run(), captured by the viewer.
      </p>
    </section>
  )
}

/**
 * One call of a run: what it returned, formatted as the script got it where that can be
 * recovered, or Neo's recorded result, and the sentence that says which. Its title places it:
 * `page.ariaSnapshot · run 8 · line 7 · call 2 of 3`.
 */
function OperationPanel({
  run,
  selection,
  onShowReply,
  onOpenFull,
}: {
  run: RunSummary
  selection: OperationSelection
  onShowReply: () => void
  onOpenFull: () => void
}) {
  const [view, setView] = useState<ReplyView>('formatted')
  const fetched = useRunDetail(run, true)
  const detail = fetched?.id === run.id ? fetched : null
  const operations = detail?.neo?.operations ?? []
  const operation =
    operations.find((candidate) => candidate.id === selection.operationId) ??
    null
  const output = operation ? operationOutput(operation, operations) : null
  const formatted = view === 'formatted'
  const lines = output ? (formatted ? output.formatted : output.raw) : []
  const text = output ? (formatted ? output.formattedText : output.rawText) : ''
  const title = [
    operation?.name ?? 'Call',
    `run ${run.seq}`,
    placeText(selection.place),
    ...(operation && isNeoInternal(operation) ? ['Neo internal'] : []),
  ].join(' · ')
  return (
    <section
      aria-label="What the call returned"
      className="flex min-h-[280px] flex-1 flex-col gap-2"
    >
      <div className="flex items-center gap-2">
        <h2 className="min-w-0 truncate font-semibold text-[13px] text-ink leading-4">
          {title}
        </h2>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onOpenFull}
          disabled={!output}
          className="flex shrink-0 items-center gap-[5px] rounded-[5px] border border-control px-2 py-[3px] font-medium text-ink text-xs leading-4 hover:bg-panel disabled:opacity-50"
        >
          <OpenIcon />
          Open full
        </button>
      </div>
      <div className="flex items-center gap-2.5">
        <Switch
          label="Call view"
          options={REPLY_VIEWS}
          value={view}
          onChange={setView}
          size="small"
        />
        <span className="min-w-0 truncate text-[11px] text-ink-2 leading-[14px]">
          {output
            ? `${count(text.length)} chars, ≈${count(Math.floor(text.length / 4))} tok, ${plural(lines.length, 'line')}`
            : detail
              ? "This call is not in Neo's record."
              : 'Loading…'}
        </span>
      </div>
      <OutputLines lines={lines} />
      <p className="shrink-0 text-[11px] text-ink-2 leading-4">
        {formatted ? output?.source : "Neo's record of the result, as JSON."}
        {output?.cut && ' Neo kept only the start of it.'}{' '}
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

function OutputLines({ lines }: { lines: Line[] }) {
  return (
    <div className="min-h-0 flex-1 overflow-auto rounded-[5px] bg-panel">
      <pre className="w-max min-w-full px-3 py-2.5 font-code text-[11px] text-ink leading-[15px]">
        {lines.map((line, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: an output line is its line number.
          <div key={index}>
            <LineSpans line={line} />
            {line.length === 0 && ' '}
          </div>
        ))}
      </pre>
    </div>
  )
}

/** The height of `element`'s content box (its visible height less its padding), as it changes. */
function useContentHeight(
  element: RefObject<HTMLElement | null>,
): number | null {
  const [height, setHeight] = useState<number | null>(null)
  useLayoutEffect(() => {
    const target = element.current
    if (!target) return
    const measure = () => {
      const style = getComputedStyle(target)
      setHeight(
        target.clientHeight -
          Number.parseFloat(style.paddingTop) -
          Number.parseFloat(style.paddingBottom),
      )
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(target)
    return () => observer.disconnect()
  }, [element])
  return height
}
