/**
 * One run's row in the run table: its number and start, a one-line summary of its script (or,
 * open, of what its block holds), its status, the Neo-time bar and the token columns.
 */

import type { RunDetail, RunSummary } from '../../shared/api'
import type { FormattedCode } from '../../shared/code-format'
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
  code,
  detail,
  scale,
  open,
  selected,
  onClick,
  nowMs,
}: {
  run: RunSummary
  /** The script laid out (`shared/code-format.ts`), and its first statement on one line. */
  code: { formatted: FormattedCode; first: string }
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
        <ClientLabel run={run} />
        <TaskChips run={run} all={open} />
        {open ? (
          <OpenSummary
            run={run}
            lines={code.formatted.lines.length}
            detail={detail}
          />
        ) : (
          <ClosedSummary
            run={run}
            first={code.first}
            lines={code.formatted.lines.length}
          />
        )}
      </span>
      <span
        className={`w-10 shrink-0 text-right ${run.operationCount === null ? 'text-ink-4' : ''}`}
        title="Calls Neo recorded for the run: the script's own and Neo's (events, frames)"
      >
        {run.operationCount === null ? '–' : count(run.operationCount)}
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
      {/* The copy slot: `CopyRunButton` lies over it, beside this button rather than in it. */}
      <span className="w-8 shrink-0" />
    </button>
  )
}

/** A closed row: the script's first statement on one line, its length, and why it failed. */
function ClosedSummary({
  run,
  first,
  lines,
}: {
  run: RunSummary
  first: string
  /** Display lines: the open block's count. */
  lines: number
}) {
  const statement = codeLines(first)[0] ?? []
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
        <LineSpans line={statement} />
      </span>
      {lines > 1 && (
        <span className="shrink-0 text-ink-3">{plural(lines, 'line')}</span>
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
  lines,
  detail,
}: {
  run: RunSummary
  lines: number
  detail: RunDetail | null
}) {
  const returned = detail ? returnedWhat(detail) : null
  return (
    <span className="min-w-0 overflow-clip font-plex">
      <span className="font-semibold">run()</span>{' '}
      <span className="text-ink-2">
        {plural(lines, 'line')} of code
        {run.operationCount !== null &&
          `, ${plural(run.operationCount, 'method call')}`}
        {run.status === 'running'
          ? ', running'
          : returned
            ? `, ${returned}`
            : ''}
      </span>
    </span>
  )
}

/** Which agent sent the run, quietly: several may share the list. */
function ClientLabel({ run }: { run: RunSummary }) {
  const name = run.client?.name
  if (!name) return null
  return (
    <span className="shrink-0 font-plex text-[11px] text-ink-4">{name}</span>
  )
}

/**
 * The tasks the run touched, as quiet chips beside the client, drawn like the methods' read/write
 * chips: the ones its script created or picked (`neo.task`), then the host's own task (the
 * request's `_meta` taskId) with a small "host" mark. A closed row shows the first and "+N"; an
 * open one shows all; the title lists all either way. A task keeps one hue on every row (from its
 * id), so the eye can follow a task down the list without a menu. A run with no task shows none.
 */
export function TaskChips({ run, all }: { run: RunSummary; all: boolean }) {
  const chips = [
    ...run.tasks.map((task) => ({ id: task.id, name: task.name, host: false })),
    ...(run.agentTaskId &&
    !run.tasks.some((task) => task.id === run.agentTaskId)
      ? [{ id: run.agentTaskId, name: null, host: true }]
      : []),
  ]
  if (chips.length === 0) return null
  const shown = all ? chips : chips.slice(0, 1)
  const title = chips
    .map(
      (chip) =>
        `${chip.host ? 'host task' : 'task'} ${chip.id}${chip.name ? ` (${chip.name})` : ''}`,
    )
    .join(', ')
  return (
    <span
      className="flex shrink-0 items-baseline gap-1"
      title={title}
      data-tasks={chips.length}
    >
      {shown.map((chip) => (
        <span
          key={`${chip.host}-${chip.id}`}
          data-task={chip.id}
          className="rounded px-[7px] py-px font-code text-[11px] leading-[14px]"
          style={taskHue(chip.id)}
        >
          {chip.host && (
            <span className="mr-1 font-plex text-[10px] opacity-70">host</span>
          )}
          {chip.id}
        </span>
      ))}
      {shown.length < chips.length && (
        <span className="font-plex text-[11px] text-ink-3">
          +{chips.length - shown.length}
        </span>
      )}
    </span>
  )
}

/** One light hue per task id, stable across rows and reloads. */
export function taskHue(id: string): { background: string; color: string } {
  let hash = 0
  for (const char of id) hash = (hash * 31 + (char.codePointAt(0) ?? 0)) >>> 0
  const hue = hash % 360
  return { background: `hsl(${hue} 70% 93%)`, color: `hsl(${hue} 45% 28%)` }
}
