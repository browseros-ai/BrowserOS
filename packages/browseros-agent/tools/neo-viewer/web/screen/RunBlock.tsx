/**
 * An open run: its script laid out one statement per line (`shared/code-format.ts` changes only
 * whitespace; the line numbers are display lines), each line with the methods it called
 * (`MethodRows`), Neo's own operations folded under the code, then "Model
 * read", the reply the model got back.
 *
 * Selecting: a method row selects that call, and the right panel shows what it returned; the
 * "Model read" row selects the reply again. The selection lives on the screen
 * (`ViewerScreen`), so the panel and "Open full" follow it; here it opens the call's detail.
 *
 * The methods come from Neo's record of the run (`RunDetail.neo`), read once the reply is in. A
 * run still running, or one whose record could not be read (no host API, image stopped), shows
 * its code alone with a note saying why.
 */

import { useState } from 'react'
import type { NeoOperation, RunDetail, RunSummary } from '../../shared/api'
import type { FormattedCode } from '../../shared/code-format'
import {
  MethodDetail,
  MethodHeader,
  MethodRow,
  Rail,
  UnplacedGroup,
} from './MethodRows'
import { type OperationOutput, operationOutput } from './operation-output'
import {
  type CallPlace,
  callPlaces,
  isNeoInternal,
  type OperationSelection,
  ordinal,
  type PlacedOperations,
  placeOperations,
} from './operations'
import { noReplyReason, replyFacts, valueChip } from './reply'
import { codeLines, type Line } from './text-lines'
import { plural } from './units'

/**
 * A line that called more methods than this (a loop) shows the first `SHOWN_PER_LINE` and folds
 * the rest behind one row. Up to `FOLD_AFTER` all show: folding a single call would save no room.
 */
const SHOWN_PER_LINE = 3
const FOLD_AFTER = SHOWN_PER_LINE + 1

export function RunBlock({
  run,
  formatted,
  detail,
  selected,
  selectedOperationId,
  onSelect,
  onOpenFull,
}: {
  run: RunSummary
  /** The script laid out by `shared/code-format.ts`; sites map to its lines. */
  formatted: FormattedCode
  detail: RunDetail | null
  selected: boolean
  /** The call the right panel shows, when it is one of this run's. */
  selectedOperationId: string | null
  /** Select a call, or null for the run's reply. */
  onSelect: (selection: OperationSelection | null) => void
  onOpenFull: () => void
}) {
  // Laid out one statement per line (whitespace only); sites map to the line that holds them.
  const lines = codeLines(formatted.text)
  const record = detail?.neo ?? null
  const placed = record
    ? placeOperations(record, lines.length, formatted.lineOfSite)
    : null
  const places = placed ? callPlaces(placed) : new Map<string, CallPlace>()
  const operations = record?.operations ?? []
  const outputOf = (operation: NeoOperation) =>
    operationOutput(operation, operations)
  const openId = selectedOperationId
  const [unfolded, setUnfolded] = useState<ReadonlySet<number>>(new Set())
  const [showUnplaced, setShowUnplaced] = useState(false)
  const toggle = (id: string) =>
    onSelect(
      id === openId
        ? null
        : {
            runId: run.id,
            operationId: id,
            place: places.get(id) ?? { line: null, call: 1, calls: 1 },
          },
    )

  return (
    <div
      className={`flex flex-col border-neo-edge border-b pt-1 pb-1.5 text-xs leading-4 ${selected ? 'bg-neo-tint-2' : 'bg-neo-tint-3'}`}
    >
      {placed && <MethodHeader />}
      {lines.map((line, index) => (
        <CodeLine
          // biome-ignore lint/suspicious/noArrayIndexKey: a script line is its line number.
          key={index}
          number={index + 1}
          line={line}
          placed={placed}
          places={places}
          outputOf={outputOf}
          note={index === 0 && !placed ? missingRecord(run, detail) : null}
          openId={openId}
          onToggle={toggle}
          unfolded={unfolded.has(index + 1)}
          onUnfold={() => setUnfolded(new Set(unfolded).add(index + 1))}
        />
      ))}
      {placed && placed.unplaced.length > 0 && (
        <UnplacedGroup
          operations={placed.unplaced}
          spanMs={placed.spanMs}
          expanded={showUnplaced}
          onExpand={() => setShowUnplaced(!showUnplaced)}
          openId={openId}
          onToggle={toggle}
          outputOf={outputOf}
        />
      )}
      <div
        className={`flex h-7 shrink-0 items-center px-4 ${selected && openId === null ? 'bg-[#DCE6FB]' : ''}`}
      >
        <Rail />
        <button
          type="button"
          onClick={() => onSelect(null)}
          title="Show the reply in the right panel"
          className="w-[106px] shrink-0 text-left font-semibold text-black hover:text-neo"
        >
          Model read
        </button>
        <ModelRead
          run={run}
          detail={detail}
          selected={selected && openId === null}
          onOpenFull={onOpenFull}
        />
      </div>
    </div>
  )
}

/** Why a block has no methods: the run is still going, or Neo's record is not there. */
function missingRecord(run: RunSummary, detail: RunDetail | null): string {
  if (run.status === 'running')
    return "Methods appear when the run ends and Neo's record is read."
  if (!detail) return ''
  return 'No record from Neo for this run (no host API, or not read yet), so no methods.'
}

/**
 * One script line: its first method beside the code, more methods on rows under it. When the
 * line called its first method more than once (a loop), that row counts the calls and each
 * later call of a method says which it is; Neo's own bookkeeping says so instead.
 */
function CodeLine({
  number,
  line,
  placed,
  places,
  outputOf,
  note,
  openId,
  onToggle,
  unfolded,
  onUnfold,
}: {
  number: number
  line: Line
  placed: PlacedOperations | null
  places: Map<string, CallPlace>
  outputOf: (operation: NeoOperation) => OperationOutput
  note: string | null
  openId: string | null
  onToggle: (id: string) => void
  unfolded: boolean
  onUnfold: () => void
}) {
  const operations = placed?.byLine.get(number) ?? []
  const spanMs = placed?.spanMs ?? 1
  const [first, ...more] = operations
  const shown =
    unfolded || operations.length <= FOLD_AFTER
      ? more
      : more.slice(0, SHOWN_PER_LINE - 1)
  const folded = more.length - shown.length
  const row = (operation: NeoOperation, isFirst: boolean) => {
    const place = places.get(operation.id)
    return (
      <MethodOnLine
        key={operation.id}
        operation={operation}
        outputOf={outputOf}
        number={isFirst ? number : undefined}
        code={isFirst ? line : undefined}
        calls={isFirst ? place?.calls : undefined}
        label={isFirst ? undefined : rowLabel(operation, place)}
        spanMs={spanMs}
        open={operation.id === openId}
        onToggle={() => onToggle(operation.id)}
      />
    )
  }
  return (
    <>
      {first ? (
        row(first, true)
      ) : (
        <MethodRow
          number={number}
          code={line}
          operation={null}
          spanMs={spanMs}
          open={false}
          aside={note}
        />
      )}
      {shown.map((operation) => row(operation, false))}
      {folded > 0 && (
        <button
          type="button"
          onClick={onUnfold}
          className="flex min-h-6 w-full shrink-0 items-start px-4 py-1 text-left text-[11px] text-ink-3 leading-[14px] hover:bg-neo-tint"
        >
          <Rail bleed />
          <span className="w-[106px] shrink-0" />
          <span className="pt-px">
            {plural(folded, 'more call')} on this line: show
          </span>
        </button>
      )}
    </>
  )
}

/** A later row's words: Neo's bookkeeping, or which call of its method it is (`2nd call`). */
function rowLabel(
  operation: NeoOperation,
  place: CallPlace | undefined,
): string | undefined {
  if (isNeoInternal(operation)) return 'Neo internal'
  return place && place.call > 1 ? `${ordinal(place.call)} call` : undefined
}

function MethodOnLine({
  operation,
  outputOf,
  number,
  code,
  calls,
  label,
  spanMs,
  open,
  onToggle,
}: {
  operation: NeoOperation
  /** Its output, worked out only while the row is open. */
  outputOf: (operation: NeoOperation) => OperationOutput
  number?: number
  code?: Line
  calls?: number
  label?: string
  spanMs: number
  open: boolean
  onToggle: () => void
}) {
  return (
    <>
      <MethodRow
        number={number}
        code={code}
        calls={calls}
        label={label}
        operation={operation}
        spanMs={spanMs}
        open={open}
        onToggle={onToggle}
      />
      {open && (
        <MethodDetail operation={operation} output={outputOf(operation)} />
      )}
    </>
  )
}

function ModelRead({
  run,
  detail,
  selected,
  onOpenFull,
}: {
  run: RunSummary
  detail: RunDetail | null
  selected: boolean
  onOpenFull: () => void
}) {
  if (run.status === 'running') {
    return (
      <span className="text-[11px] text-ink-2">Waiting for Neo's reply.</span>
    )
  }
  if (!detail) return <span className="text-[11px] text-ink-3">Loading…</span>
  const text = detail.reply.text
  if (text === null || !run.replySize) {
    return (
      <span className="text-[11px] text-ink-2">{noReplyReason(detail)}</span>
    )
  }
  const chip = valueChip(text)
  return (
    <span className="flex min-w-0 flex-1 items-center gap-2.5">
      <code
        className={`min-w-0 max-w-[60%] truncate rounded border border-neo-edge bg-white px-2 py-0.5 font-code ${chip.failed ? 'text-bad' : 'text-ink'}`}
      >
        {chip.text}
      </code>
      <span className="shrink-0 whitespace-pre text-[11px] text-ink-2 leading-[14px]">
        {replyFacts(text, run.replySize)}
        {selected && ', shown in full on the right'}
      </span>
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
