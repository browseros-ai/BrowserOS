/**
 * Copy for one run: its Markdown (code, each line's methods, the error, and all the model read),
 * to paste into any agent and ask about it. The block comes from `shared/run-markdown.ts`, the
 * renderer the export's `runs/NNN.md` uses, so the two never drift; Copy shows the whole reply.
 *
 * A closed row has no detail loaded, so Copy fetches it. The clipboard is handed a promise of the
 * text (`ClipboardItem`) at the click, so the write keeps the click's user activation however long
 * the fetch takes; a browser without `ClipboardItem` gets `writeText` once the text is ready.
 *
 * The button is a sibling of the row's button, laid over the row's end (`RunRow`'s copy slot),
 * not inside it: a button cannot hold a button, and a click on Copy never opens or closes the row.
 */

import { useEffect, useState } from 'react'
import type { RunDetail, RunResponse, RunSummary } from '../../shared/api'
import { apiRoutes } from '../../shared/api'
import { runMarkdown } from '../../shared/run-markdown'

/** The run's Markdown as Copy writes it: the shared renderer, with no limit on the reply. */
export function runCopyText(detail: RunDetail): string {
  return runMarkdown(detail, { replyChars: null })
}

async function fetchDetail(runId: string): Promise<RunDetail> {
  const response = await fetch(apiRoutes.run(runId))
  if (!response.ok)
    throw new Error(`${apiRoutes.run(runId)}: HTTP ${response.status}`)
  return ((await response.json()) as RunResponse).run
}

/** Writes the run's Markdown to the clipboard, fetching the run first when its row is closed. */
export async function copyRun(
  run: RunSummary,
  detail: RunDetail | null,
): Promise<void> {
  const text = (detail ? Promise.resolve(detail) : fetchDetail(run.id)).then(
    runCopyText,
  )
  if (typeof ClipboardItem === 'undefined') {
    await navigator.clipboard.writeText(await text)
    return
  }
  await navigator.clipboard.write([
    new ClipboardItem({
      'text/plain': text.then(
        (markdown) => new Blob([markdown], { type: 'text/plain' }),
      ),
    }),
  ])
}

/** How long the tick and "Copied" stay. */
const COPIED_MS = 1_500

type CopyState = 'idle' | 'copied' | 'failed'

export function CopyRunButton({
  run,
  detail,
  open,
}: {
  run: RunSummary
  detail: RunDetail | null
  /** An open row always shows Copy; a closed one on hover and keyboard focus. */
  open: boolean
}) {
  const [state, setState] = useState<CopyState>('idle')
  useEffect(() => {
    if (state === 'idle') return
    const timer = setTimeout(() => setState('idle'), COPIED_MS)
    return () => clearTimeout(timer)
  }, [state])
  const label = `Copy run ${run.seq} as Markdown: its code, the methods each line called, and what the model read`
  return (
    <span
      className={`absolute top-[3px] right-4 flex items-center gap-1 ${
        open || state !== 'idle'
          ? 'opacity-100'
          : 'opacity-0 group-focus-within:opacity-100 group-hover:opacity-100'
      }`}
    >
      {state !== 'idle' && (
        <span
          role="status"
          className={`rounded bg-white px-1 font-plex text-[11px] leading-[14px] ${state === 'copied' ? 'text-ok' : 'text-bad'}`}
        >
          {state === 'copied' ? 'Copied' : 'Copy failed'}
        </span>
      )}
      <button
        type="button"
        data-copy-run={run.seq}
        aria-label={label}
        title={label}
        onClick={() => {
          copyRun(run, detail).then(
            () => setState('copied'),
            () => setState('failed'),
          )
        }}
        className="flex size-6 items-center justify-center rounded-md border border-control bg-white text-ink-2 hover:bg-[#F4F6FA] hover:text-ink focus-visible:opacity-100"
      >
        {state === 'copied' ? <TickIcon /> : <CopyIcon />}
      </button>
    </span>
  )
}

function CopyIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
      <rect
        x="3.5"
        y="3.5"
        width="7"
        height="7"
        rx="1.2"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
      />
      <path
        d="M8.5 1.5h-6a1 1 0 0 0-1 1v6"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
      />
    </svg>
  )
}

function TickIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
      <path
        d="M2 6.5l2.5 2.5L10 3.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      />
    </svg>
  )
}
