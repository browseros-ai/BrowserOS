/**
 * The runs: one row per `run()` call, with its Neo-time bar, round trip and token columns, sortable
 * by any of them, and a totals row. Opening a row shows its code block and "Model read" under it
 * (`RunBlock`); the selected row's reply is the one the sidebar shows.
 *
 * Every bar is a share of the largest value in its column (`measures.ts`), so one glance across
 * the rows answers "which call was expensive".
 */

import type { RunSummary, TextSize } from '../../shared/api'
import { useRunDetail } from '../use-viewer'
import { type BarScale, barScale, type RunTotals } from './measures'
import { RunBlock } from './RunBlock'
import { RunRow } from './RunRow'
import { nextSort, type RunSort, type SortKey, sortRuns } from './sort'
import { Approx } from './ui'
import { count, plural, seconds } from './units'

export interface RunTableProps {
  runs: RunSummary[]
  totals: RunTotals
  toolsList: TextSize | null
  sort: RunSort
  onSort: (sort: RunSort) => void
  openIds: ReadonlySet<string>
  selectedId: string | null
  onRow: (run: RunSummary) => void
  onOpenFull: (run: RunSummary) => void
  nowMs: number
}

export function RunTable(props: RunTableProps) {
  const { runs, sort } = props
  const scale = barScale(runs)
  return (
    <div className="flex min-w-[720px] flex-col">
      <div className="sticky top-0 z-10">
        <Header sort={sort} onSort={props.onSort} />
        <Legend />
      </div>
      {runs.length === 0 ? (
        <p className="border-line-2 border-b px-4 py-6 text-ink-2 text-xs leading-4">
          No runs in this task yet. Each run() call the agent makes appears here
          as it starts, and fills in when Neo answers.
        </p>
      ) : (
        sortRuns(runs, sort).map((run) => (
          <RunEntry
            key={run.id}
            run={run}
            scale={scale}
            open={props.openIds.has(run.id)}
            selected={run.id === props.selectedId}
            onRow={props.onRow}
            onOpenFull={props.onOpenFull}
            nowMs={props.nowMs}
          />
        ))
      )}
      {runs.length > 0 && (
        <TotalsRow totals={props.totals} toolsList={props.toolsList} />
      )}
    </div>
  )
}

function Header({
  sort,
  onSort,
}: {
  sort: RunSort
  onSort: (sort: RunSort) => void
}) {
  const column = (
    key: SortKey,
    label: string,
    className: string,
    title?: string,
  ) => {
    const active = sort.key === key
    return (
      <button
        type="button"
        onClick={() => onSort(nextSort(sort, key))}
        aria-label={`Sort by ${label}`}
        title={title}
        aria-pressed={active}
        className={`shrink-0 text-[11px] leading-[14px] hover:text-ink ${active ? 'font-semibold text-ink' : 'font-medium text-ink-2'} ${className}`}
      >
        {label}
        {active && (sort.direction === 'asc' ? ' ▴' : ' ▾')}
      </button>
    )
  }
  return (
    <div className="flex h-[30px] items-center border-line border-b bg-panel px-4">
      {column('seq', '#', 'w-[34px] text-left')}
      <span className="w-[84px] shrink-0 font-medium text-[11px] text-ink-2 leading-[14px]">
        Started
      </span>
      <span className="min-w-0 flex-1 truncate font-medium text-[11px] text-ink-2 leading-[14px]">
        run() code block, and the methods each line called
      </span>
      <span className="w-10 shrink-0 text-right font-medium text-[11px] text-ink-2 leading-[14px]">
        Calls
      </span>
      <span className="w-[22px] shrink-0" />
      {column(
        'neo',
        'Neo time',
        'w-[170px] text-left',
        "Measured at the viewer: the round trip less the time the viewer held the call. Neo's own record joins with the next PR.",
      )}
      {column('roundTrip', 'Round trip', 'w-[70px] text-right')}
      {column('code', 'Code tok', 'w-[62px] text-right')}
      {column('reply', 'Reply tok', 'w-[62px] text-right')}
      <span className="w-4 shrink-0" />
      <span className="w-[110px] shrink-0 font-medium text-[11px] text-ink-2 leading-[14px]">
        <span className="text-neo-soft">■</span> code{' '}
        <span className="text-neo">■</span> reply
      </span>
    </div>
  )
}

function Legend() {
  return (
    <div className="flex h-6 items-center gap-[18px] overflow-hidden whitespace-nowrap border-line border-b bg-panel px-4 text-[11px] text-ink-2 leading-[14px]">
      <span>
        <b className="font-semibold text-ink">Code tok</b>: the script the agent
        sent in run(), what the model wrote.
      </span>
      <span>
        <b className="font-semibold text-ink">Reply tok</b>: what run() returned
        into the agent's context, what the model had to read.
      </span>
      <span>Both ≈ chars ÷ 4.</span>
    </div>
  )
}

/** One run's row and, when open, its code block; the block's reply is fetched once for both. */
function RunEntry({
  run,
  scale,
  open,
  selected,
  onRow,
  onOpenFull,
  nowMs,
}: {
  run: RunSummary
  scale: BarScale
  open: boolean
  selected: boolean
  onRow: (run: RunSummary) => void
  onOpenFull: (run: RunSummary) => void
  nowMs: number
}) {
  const fetched = useRunDetail(run, open)
  // The hook keeps the last run it fetched; only this run's own detail counts.
  const detail = fetched?.id === run.id ? fetched : null
  return (
    <>
      <RunRow
        run={run}
        detail={detail}
        scale={scale}
        open={open}
        selected={selected}
        onClick={() => onRow(run)}
        nowMs={nowMs}
      />
      {open && (
        <RunBlock
          run={run}
          detail={detail}
          selected={selected}
          onOpenFull={() => onOpenFull(run)}
        />
      )}
    </>
  )
}

function TotalsRow({
  totals,
  toolsList,
}: {
  totals: RunTotals
  toolsList: TextSize | null
}) {
  return (
    <div className="flex h-[30px] shrink-0 items-center border-line border-b bg-panel px-4 text-black text-xs leading-4">
      <span className="w-[118px] shrink-0 font-semibold">
        {plural(totals.runs, 'run')}
      </span>
      <span className="min-w-0 flex-1 truncate text-ink-2">
        {toolsList && (
          <>
            plus{' '}
            <span className="font-code text-ink">
              ≈{count(toolsList.tokens)}
            </span>{' '}
            tok of tool description, read once per task
          </>
        )}
      </span>
      <span className="w-10 shrink-0 text-right font-code text-ink-4">–</span>
      <span className="w-[22px] shrink-0" />
      <span className="w-[170px] shrink-0 pl-[118px] font-code font-semibold">
        {seconds(totals.neoMs)}
      </span>
      <span className="w-[70px] shrink-0 text-right font-code text-ink-2">
        {seconds(totals.roundTripMs)}
      </span>
      <span className="w-[62px] shrink-0 text-right font-code font-semibold">
        <Approx />
        {count(totals.codeTokens)}
      </span>
      <span className="w-[62px] shrink-0 text-right font-code font-semibold">
        <Approx />
        {count(totals.replyTokens)}
      </span>
      <span className="w-4 shrink-0" />
      <span className="w-[110px] shrink-0 text-ink-2">
        <span className="font-code font-semibold text-neo">
          ≈{count(totals.codeTokens + totals.replyTokens)}
        </span>{' '}
        tok
      </span>
    </div>
  )
}
