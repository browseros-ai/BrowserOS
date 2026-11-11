/**
 * The right sidebar. On top, where the design docks the live screen, a placeholder of the same
 * size: the screen comes with Neo's display relay (PR C). Under it, "What the model read": the
 * selected run's full reply exactly as the proxy captured it, formatted or raw.
 */

import { useState } from 'react'
import type { RunSummary } from '../../shared/api'
import { useRunDetail } from '../use-viewer'
import { formattedLines, noReplyReason, rawLines, replyFacts } from './reply'
import { LineSpans, OpenIcon, REPLY_VIEWS, type ReplyView, Switch } from './ui'

export function Sidebar({
  width,
  run,
  onOpenFull,
}: {
  width: number
  run: RunSummary | null
  onOpenFull: (run: RunSummary) => void
}) {
  return (
    <aside
      aria-label="Sidebar"
      style={{ width }}
      className="flex min-h-0 shrink-0 flex-col gap-3 px-4 py-3.5"
    >
      <section className="flex shrink-0 flex-col gap-2 border-line border-b pb-3">
        <div className="flex h-[22px] items-center">
          <h2 className="font-semibold text-[13px] text-ink leading-4">
            Live screen
          </h2>
        </div>
        <div className="flex h-[200px] flex-col items-center justify-center gap-1 rounded-md border border-control border-dashed bg-panel-2 px-6 text-center">
          <p className="font-medium text-ink text-xs leading-4">
            The live screen arrives with PR C
          </p>
          <p className="text-[11px] text-ink-2 leading-[14px]">
            Neo's display, following the agent or under your control, docks
            here.
          </p>
        </div>
      </section>
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
      className="flex min-h-0 flex-1 flex-col gap-2"
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
