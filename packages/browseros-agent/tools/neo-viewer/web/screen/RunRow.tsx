/**
 * One run's row in the run table: its number and start, a one-line summary of its script (or,
 * open, of what its block holds), its status, the Neo-time bar and the token columns.
 */

import type { RunDetail, RunSummary } from '../../shared/api'
import { type BarScale, failed, neoMs, share, tokenBar } from './measures'
import { errorCode, returnedWhat } from './reply'
import { codeLines } from './text-lines'
import { Approx, LineSpans, StatusDot } from './ui'
import { clock, count, plural, seconds } from './units'

function rowTone(run: RunSummary, open: boolean, selected: boolean): string {
  if (selected) return 'border-neo-edge bg-neo-tint'
  if (open) return 'border-line bg-neo-tint-3'
  if (run.status === 'running') return 'border-line-2 bg-busy-tint'
  return 'border-line-2 hover:bg-panel-2'
}

export function RunRow({
  run,
  detail,
  scale,
  open,
  selected,
  onClick,
  nowMs,
}: {
  run: RunSummary
  detail: RunDetail | null
  scale: BarScale
  open: boolean
  selected: boolean
  onClick: () => void
  nowMs: number
}) {
  const running = run.status === 'running'
  const neo = neoMs(run)
  const bar = tokenBar(run, scale)
  return (
    <button
      type="button"
      onClick={onClick}
      aria-expanded={open}
      aria-current={selected ? 'true' : undefined}
      data-run={run.seq}
      className={`flex h-[30px] w-full shrink-0 items-center border-b px-4 text-left font-code text-black text-xs leading-4 ${rowTone(run, open, selected)}`}
    >
      <span
        className={`w-[34px] shrink-0 ${selected ? 'font-semibold text-neo' : open ? 'font-semibold text-ink' : running ? 'text-busy-ink' : 'text-ink-2'}`}
      >
        {open ? '▾' : '▸'} {run.seq}
      </span>
      <span className="w-[84px] shrink-0 text-ink-2">
        {clock(run.sentAt, true)}
      </span>
      <span className="flex min-w-0 flex-1 items-baseline gap-[1ch] whitespace-pre pr-3">
        {open ? (
          <OpenSummary run={run} detail={detail} />
        ) : (
          <ClosedSummary run={run} />
        )}
      </span>
      <span
        className="w-10 shrink-0 text-right text-ink-4"
        title="Method calls per run arrive with Neo's operations (next PR)"
      >
        –
      </span>
      <span className="flex w-[22px] shrink-0 justify-center">
        <StatusDot status={run.status} />
      </span>
      <span className="flex w-[170px] shrink-0 items-center gap-2">
        <span className="flex h-2.5 w-[110px] shrink-0">
          {running ? (
            <span className="block h-2.5 w-full border border-busy border-dashed" />
          ) : (
            neo !== null && (
              <span
                data-bar="neo"
                className={`block h-2.5 ${failed(run) ? 'bg-bad' : run.status === 'ok' ? 'bg-ink' : 'bg-ink-4'}`}
                style={{ width: `max(2px, ${share(neo, scale.neoMs) * 100}%)` }}
              />
            )
          )}
        </span>
        <span className={running ? 'text-busy-ink' : undefined}>
          {running
            ? `${seconds(Math.max(0, nowMs - Date.parse(run.sentAt)))}…`
            : neo === null
              ? '–'
              : seconds(neo)}
        </span>
      </span>
      <span
        className={`w-[70px] shrink-0 text-right ${running ? 'text-ink-4' : 'text-ink-2'}`}
      >
        {running
          ? '…'
          : run.roundTripMs === null
            ? '–'
            : seconds(run.roundTripMs)}
      </span>
      <span className="w-[62px] shrink-0 text-right">
        <Approx />
        {count(run.codeSize.tokens)}
      </span>
      <span
        className={`w-[62px] shrink-0 text-right ${run.replySize ? '' : 'text-ink-4'}`}
      >
        {run.replySize ? (
          <>
            <Approx />
            {count(run.replySize.tokens)}
          </>
        ) : running ? (
          '…'
        ) : (
          '–'
        )}
      </span>
      <span className="w-4 shrink-0" />
      <span className="flex h-2.5 w-[110px] shrink-0">
        <span
          data-bar="code"
          className="block h-2.5 bg-neo-soft"
          style={{ width: `${bar.code * 100}%` }}
        />
        <span
          data-bar="reply"
          className="block h-2.5 bg-neo"
          style={{ width: `${bar.reply * 100}%` }}
        />
      </span>
    </button>
  )
}

function lineCount(code: string): number {
  return code.trimEnd().split('\n').length
}

/** A closed row: the script's first line, its length, and why it failed if it did. */
function ClosedSummary({ run }: { run: RunSummary }) {
  const lines = codeLines(run.code.trim())
  const first = lines[0] ?? []
  const total = lineCount(run.code)
  const failure =
    run.status === 'error'
      ? (errorCode(run.replyPreview) ?? 'error')
      : run.status === 'ok' || run.status === 'running'
        ? null
        : run.status
  // The code clips; its length and the failure stay visible after it.
  return (
    <>
      <span className="min-w-0 truncate">
        <LineSpans line={first} />
      </span>
      {total > 1 && (
        <span className="shrink-0 text-ink-3">{plural(total, 'line')}</span>
      )}
      {failure && (
        <span className="shrink-0 font-plex text-[11px] text-bad leading-[14px]">
          {failure}
        </span>
      )}
    </>
  )
}

/** An open row: what the block below holds, in words. */
function OpenSummary({
  run,
  detail,
}: {
  run: RunSummary
  detail: RunDetail | null
}) {
  const returned = detail ? returnedWhat(detail) : null
  return (
    <span className="min-w-0 overflow-clip font-plex">
      <span className="font-semibold">run()</span>{' '}
      <span className="text-ink-2">
        {plural(lineCount(run.code), 'line')} of code
        {run.status === 'running'
          ? ', running'
          : returned
            ? `, ${returned}`
            : ''}
      </span>
    </span>
  )
}
