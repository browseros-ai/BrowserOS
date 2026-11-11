/**
 * The methods inside an open run's code block (artboard "Viewer C v2: Run blocks with their
 * methods"). Each method sits in four lanes to the right of its code line: its chip (the name Neo
 * recorded, tinted by read or write), its outcome, a waterfall bar inside the run's time with its
 * duration, and read/write. Its start offset sits left of the line number. Opening a method shows
 * its arguments, call log, result, outcome and error from Neo's record.
 */

import type { ReactNode } from 'react'
import type { NeoOperation } from '../../shared/api'
import { argumentLines, resultLines } from './operation-values'
import { namesWithCounts, waterfallBar } from './operations'
import type { Line } from './text-lines'
import { LineSpans } from './ui'
import { plural, seconds } from './units'

/**
 * The blue line down the left of an open block, tying its lines to the row above. `bleed` covers
 * a padded row's padding so the line stays unbroken.
 */
export function Rail({ bleed = false }: { bleed?: boolean }) {
  return (
    <span
      className={`flex w-[34px] shrink-0 self-stretch pl-1 ${bleed ? '-my-1' : ''}`}
    >
      <span className="block w-0.5 bg-neo" />
    </span>
  )
}

const OUTCOME_DOT: Record<string, string> = {
  ok: 'bg-ok',
  error: 'bg-bad',
  cancelled: 'bg-ink-4',
}

function offsetText(operation: NeoOperation): string {
  return `+${seconds(operation.startOffsetMs)}`
}

/**
 * One row of a block: the offset, the line number and code (empty on a line's second and later
 * methods), and the method's lanes. A row with a method is a button that opens its detail.
 */
export function MethodRow({
  number,
  code,
  operation,
  spanMs,
  open,
  onToggle,
  aside,
}: {
  number?: number
  code?: Line
  operation: NeoOperation | null
  spanMs: number
  open: boolean
  onToggle?: () => void
  /** Shown in the lanes when the row has no method (a note, or nothing). */
  aside?: ReactNode
}) {
  const body = (
    <>
      <Rail bleed />
      <span
        className={`w-[84px] shrink-0 pt-px font-code text-[11px] leading-[14px] ${open ? 'text-ink-2' : 'text-ink-3'}`}
      >
        {operation && offsetText(operation)}
      </span>
      <span
        className={`w-[22px] shrink-0 pt-px pr-2.5 text-right font-code text-[11px] leading-[14px] ${open ? 'text-neo' : 'text-ink-4'}`}
      >
        {number}
      </span>
      {/* Long lines wrap rather than clip: this block is the only place the script is shown. */}
      <code className="wrap-anywhere min-w-0 flex-1 whitespace-pre-wrap font-code text-ink">
        {code && <LineSpans line={code} />}
      </code>
      {operation ? (
        <MethodLanes operation={operation} spanMs={spanMs} open={open} />
      ) : (
        <span className="w-[412px] shrink-0 truncate pt-px pl-1 text-[11px] text-ink-3 leading-[14px]">
          {aside}
        </span>
      )}
    </>
  )
  const row = `flex min-h-6 w-full shrink-0 items-start px-4 py-1 text-left ${open ? 'bg-[#DCE6FB]' : ''}`
  if (!operation || !onToggle) return <div className={row}>{body}</div>
  return (
    <button
      type="button"
      aria-expanded={open}
      aria-label={`${operation.name}${number ? ` on line ${number}` : ''}`}
      data-operation={operation.seq}
      onClick={onToggle}
      className={`${row} hover:bg-neo-tint`}
    >
      {body}
    </button>
  )
}

function MethodLanes({
  operation,
  spanMs,
  open,
}: {
  operation: NeoOperation
  spanMs: number
  open: boolean
}) {
  const bar = waterfallBar(operation, spanMs)
  const failed = operation.outcome === 'error'
  const chip = open
    ? 'bg-neo text-white'
    : operation.effect === 'write'
      ? 'bg-[#E6ECFA] text-black'
      : 'bg-line-2 text-ink'
  return (
    <>
      <span className="flex w-[150px] shrink-0">
        <span
          className={`truncate rounded px-[7px] py-px font-code text-[11px] leading-[14px] ${chip}`}
        >
          {operation.name}
        </span>
      </span>
      <span className="flex w-[22px] shrink-0 justify-center pt-[5px]">
        <span
          title={operation.outcome ?? 'running'}
          className={`block size-1.5 rounded-full ${OUTCOME_DOT[operation.outcome ?? ''] ?? 'bg-busy'}`}
        />
      </span>
      <span className="flex w-[170px] shrink-0 items-center gap-2 pt-px">
        <span className="relative h-2 w-[110px] shrink-0">
          <span
            data-bar="waterfall"
            className={`absolute inset-y-0 block ${bar.open ? 'border border-busy border-dashed' : failed ? 'bg-bad' : open ? 'bg-neo' : 'bg-ink-2'}`}
            style={{
              left: `min(${bar.left * 100}%, calc(100% - 2px))`,
              width: `max(2px, ${bar.width * 100}%)`,
            }}
          />
        </span>
        <span
          className={`font-code text-[11px] leading-[14px] ${open ? 'font-semibold text-black' : 'text-ink'}`}
        >
          {operation.durationMs === null ? '…' : seconds(operation.durationMs)}
        </span>
      </span>
      <span className="w-[70px] shrink-0 pt-px text-right text-[11px] text-ink-3 leading-[14px]">
        {operation.effect}
      </span>
    </>
  )
}

/** The open method's arguments, call log and result, under its row. */
export function MethodDetail({ operation }: { operation: NeoOperation }) {
  const args = argumentLines(operation.name, operation.args)
  const result = resultLines(operation.result)
  return (
    <div className="flex shrink-0 bg-white px-4">
      <Rail />
      <div className="flex min-w-0 flex-1 gap-6 border-neo-edge border-b pt-2 pb-2.5 pl-[84px] text-[11px] leading-4">
        <section className="flex w-[250px] shrink-0 flex-col gap-1">
          <h4 className="font-semibold text-ink leading-[14px]">Arguments</h4>
          <Lines lines={args} empty="none" />
          {operation.argsTruncated && (
            <Truncated>
              Neo keeps an operation's arguments up to 4 KiB, so this is only
              their start.
            </Truncated>
          )}
        </section>
        <section className="flex min-w-0 flex-1 flex-col gap-1">
          <h4 className="font-semibold text-ink leading-[14px]">Call log</h4>
          {operation.notes.length === 0 ? (
            <p className="text-ink-3">No call log recorded.</p>
          ) : (
            <pre className="max-h-60 overflow-auto font-code text-ink-2">
              {operation.notes.join('\n')}
            </pre>
          )}
        </section>
        <section className="flex w-[260px] shrink-0 flex-col gap-1">
          <h4 className="font-semibold text-ink leading-[14px]">Result</h4>
          {operation.error ? (
            <div className="flex flex-col gap-0.5">
              <p className="font-code text-bad">
                {operation.error.owner} / {operation.error.code}
              </p>
              <pre className="wrap-anywhere max-h-60 overflow-auto whitespace-pre-wrap font-code text-bad">
                {operation.error.message}
              </pre>
            </div>
          ) : (
            <Lines lines={result} empty="none" />
          )}
          {operation.resultTruncated && (
            <Truncated>
              Neo keeps an operation's result up to 2 KiB, so this is only its
              start.
            </Truncated>
          )}
          <p className="text-ink-2">
            {operation.outcome ?? 'running'}
            {operation.delivery ? `, delivery ${operation.delivery}` : ''}
          </p>
        </section>
      </div>
    </div>
  )
}

function Lines({ lines, empty }: { lines: Line[]; empty: string }) {
  if (lines.length === 0) return <p className="text-ink-3">{empty}</p>
  return (
    <pre className="wrap-anywhere max-h-60 overflow-auto whitespace-pre-wrap font-code text-ink">
      {lines.map((line, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a value line is its line number.
        <div key={index}>
          <LineSpans line={line} />
        </div>
      ))}
    </pre>
  )
}

/** Neo bounds what it stores; a flagged value is only its start, and says so. */
function Truncated({ children }: { children: string }) {
  return <p className="text-busy-ink">Truncated: {children}</p>
}

/**
 * Operations with no line in the script: Neo's own work for the run (opening the task's page,
 * listening for events, reading frames) and calls made inside another operation. Folded under
 * the code by default, as one quiet row.
 */
export function UnplacedGroup({
  operations,
  spanMs,
  expanded,
  onExpand,
  openId,
  onToggle,
}: {
  operations: NeoOperation[]
  spanMs: number
  expanded: boolean
  onExpand: () => void
  openId: string | null
  onToggle: (id: string) => void
}) {
  return (
    <>
      <button
        type="button"
        aria-expanded={expanded}
        onClick={onExpand}
        className="flex min-h-6 w-full shrink-0 items-start px-4 py-1 text-left text-[11px] text-ink-3 leading-[14px] hover:bg-neo-tint"
      >
        <Rail bleed />
        <span className="w-[84px] shrink-0 pt-px">{expanded ? '▾' : '▸'}</span>
        <span className="min-w-0 flex-1 truncate pt-px">
          {plural(operations.length, 'operation')} not on a line (Neo's own work
          for the run, or calls inside another): {namesWithCounts(operations)}
        </span>
      </button>
      {expanded &&
        operations.map((operation) => (
          <MethodGroupRow
            key={operation.id}
            operation={operation}
            spanMs={spanMs}
            open={operation.id === openId}
            onToggle={() => onToggle(operation.id)}
          />
        ))}
    </>
  )
}

function MethodGroupRow({
  operation,
  spanMs,
  open,
  onToggle,
}: {
  operation: NeoOperation
  spanMs: number
  open: boolean
  onToggle: () => void
}) {
  return (
    <>
      <MethodRow
        operation={operation}
        spanMs={spanMs}
        open={open}
        onToggle={onToggle}
        code={
          operation.parentId
            ? [
                {
                  text: `inside op ${operation.parentId.split('.').at(-1)}`,
                  tone: 'muted',
                },
              ]
            : undefined
        }
      />
      {open && <MethodDetail operation={operation} />}
    </>
  )
}
