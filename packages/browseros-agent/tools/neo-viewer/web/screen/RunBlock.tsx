/**
 * An open run: its script as the agent sent it, line by line and numbered, then "Model read",
 * the reply the model got back. In the design each line also carries the methods it called with
 * their waterfall; that needs Neo's operations joined to the run (PR C), so those lanes are kept
 * empty here with a quiet note on the first line.
 */

import type { RunDetail, RunSummary } from '../../shared/api'
import { noReplyReason, replyFacts, valueChip } from './reply'
import { codeLines } from './text-lines'
import { LineSpans } from './ui'

export function RunBlock({
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
  const lines = codeLines(run.code.trimEnd())
  return (
    <div
      className={`flex flex-col border-neo-edge border-b pt-1 pb-1.5 text-xs leading-4 ${selected ? 'bg-neo-tint-2' : 'bg-neo-tint-3'}`}
    >
      {lines.map((line, index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: a script line is its line number.
          key={index}
          className="flex min-h-6 shrink-0 items-start px-4 py-1"
        >
          <Rail bleed />
          <span className="w-[84px] shrink-0" />
          <span className="w-[22px] shrink-0 pt-px pr-2.5 text-right font-code text-[11px] text-ink-4 leading-[14px]">
            {index + 1}
          </span>
          {/* Long lines wrap rather than clip: this block is the only place the script is shown. */}
          <code className="wrap-anywhere min-w-0 flex-1 whitespace-pre-wrap font-code text-ink">
            <LineSpans line={line} />
          </code>
          <span className="w-[412px] shrink-0 truncate pt-px pl-1 text-[11px] text-ink-3 leading-[14px]">
            {index === 0 &&
              'The methods each line called, and their timing, arrive with the next PR.'}
          </span>
        </div>
      ))}
      <div className="flex h-7 shrink-0 items-center px-4">
        <Rail />
        <span className="w-[106px] shrink-0 font-semibold text-black">
          Model read
        </span>
        <ModelRead
          run={run}
          detail={detail}
          selected={selected}
          onOpenFull={onOpenFull}
        />
      </div>
    </div>
  )
}

/**
 * The blue line down the left of an open block, tying its lines to the row above. `bleed` covers
 * a padded row's padding so the line stays unbroken.
 */
function Rail({ bleed = false }: { bleed?: boolean }) {
  return (
    <span
      className={`flex w-[34px] shrink-0 self-stretch pl-1 ${bleed ? '-my-1' : ''}`}
    >
      <span className="block w-0.5 bg-neo" />
    </span>
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
