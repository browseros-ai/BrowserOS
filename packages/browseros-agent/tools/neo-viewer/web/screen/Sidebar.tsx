/**
 * The right sidebar. On top, the live screen docked (PR C2's `LiveScreen`: Follow agent / Control,
 * Pop out, the screen); while it is popped out the dock says where it went. Under it, "What the
 * model read": the selected run's full reply exactly as the proxy captured it, formatted or raw.
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
import { formattedLines, noReplyReason, rawLines, replyFacts } from './reply'
import { LineSpans, OpenIcon, REPLY_VIEWS, type ReplyView, Switch } from './ui'

export function Sidebar({
  width,
  run,
  onOpenFull,
  live,
  popped,
  onPopOut,
}: {
  width: number
  run: RunSummary | null
  onOpenFull: (run: RunSummary) => void
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
      {run ? (
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
      <div className="min-h-0 flex-1 overflow-auto rounded-[5px] bg-panel">
        <pre className="w-max min-w-full px-3 py-2.5 font-code text-[11px] text-ink leading-[15px]">
          {lines.map((line, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: a reply line is its line number.
            <div key={index}>
              <LineSpans line={line} />
              {line.length === 0 && ' '}
            </div>
          ))}
        </pre>
      </div>
      <p className="shrink-0 text-[11px] text-ink-2 leading-4">
        Exactly the text the agent got back from run(), captured by the viewer.
        Neo's own record keeps only the first 2 KiB.
      </p>
    </section>
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
