import { useState } from 'react'
import type { RunDetail, RunStatus, RunSummary } from '../../shared/api'
import { decodedNote, returnedString } from '../../shared/run-markdown'
import { clockTime, duration, prettyJson, tokens } from '../format'
import { useRunDetail } from '../use-viewer'

const STATUS_STYLE: Record<RunStatus, string> = {
  running: 'bg-blue-100 text-blue-800',
  ok: 'bg-green-100 text-green-800',
  error: 'bg-red-100 text-red-800',
  'rpc-error': 'bg-red-100 text-red-800',
  'http-error': 'bg-red-100 text-red-800',
  cancelled: 'bg-zinc-200 text-zinc-700',
  lost: 'bg-zinc-200 text-zinc-700',
}

/** The task's runs in the order the agent sent them; open one to read its reply. */
export function RunList({ runs }: { runs: RunSummary[] }) {
  if (runs.length === 0)
    return <p className="text-zinc-600">No runs in this task yet.</p>
  return (
    <ol className="space-y-2">
      {runs.map((run) => (
        <RunRow key={run.id} run={run} />
      ))}
    </ol>
  )
}

function RunRow({ run }: { run: RunSummary }) {
  const [open, setOpen] = useState(false)
  const detail = useRunDetail(run, open)
  return (
    <li className="rounded border border-zinc-200">
      <button
        type="button"
        className="flex w-full flex-wrap items-center gap-x-4 gap-y-1 px-3 py-2 text-left hover:bg-zinc-50"
        onClick={() => setOpen(!open)}
      >
        <span className="w-8 font-mono text-zinc-500">#{run.seq}</span>
        <span
          className={`rounded px-1.5 py-0.5 font-medium text-xs ${STATUS_STYLE[run.status]}`}
        >
          {run.status}
        </span>
        <span className="text-zinc-600">{clockTime(run.sentAt)}</span>
        <span>gap before {duration(run.gapMs)}</span>
        <span>round trip {duration(run.roundTripMs)}</span>
        <span>code {tokens(run.codeSize)}</span>
        <span>reply {tokens(run.replySize)}</span>
        {run.client && <span className="text-zinc-500">{run.client.name}</span>}
        {run.tasks.length > 0 && (
          <span className="text-zinc-500">
            {run.tasks.length === 1 ? 'task' : 'tasks'}{' '}
            {run.tasks.map((task) => task.id).join(', ')}
          </span>
        )}
        {run.agentTaskId && (
          <span className="text-zinc-500">host task {run.agentTaskId}</span>
        )}
      </button>
      <div className="space-y-2 border-zinc-200 border-t px-3 py-2">
        <pre className="overflow-x-auto whitespace-pre-wrap rounded bg-zinc-50 p-2 font-mono text-xs">
          {run.code}
        </pre>
        {open ? (
          detail ? (
            <Reply detail={detail} />
          ) : (
            <p className="text-zinc-500">Loading…</p>
          )
        ) : (
          run.replyPreview !== null && (
            <p className="truncate font-mono text-xs text-zinc-600">
              {previewText(run.replyPreview)}
            </p>
          )
        )}
      </div>
    </li>
  )
}

type ReplyView = 'formatted' | 'raw' | 'request'

function Reply({ detail }: { detail: RunDetail }) {
  const [view, setView] = useState<ReplyView>('formatted')
  const { reply } = detail
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        {(['formatted', 'raw', 'request'] as const).map((option) => (
          <button
            key={option}
            type="button"
            className={`rounded border px-2 py-0.5 ${view === option ? 'border-zinc-900 bg-zinc-900 text-white' : 'border-zinc-300'}`}
            onClick={() => setView(option)}
          >
            {option === 'formatted'
              ? 'Reply'
              : option === 'raw'
                ? 'Reply, raw'
                : 'Request, raw'}
          </button>
        ))}
        <span className="text-zinc-500">
          HTTP {reply.httpStatus ?? '–'} · proxy held {duration(detail.proxyMs)}{' '}
          · {reply.keepAlives} keep-alives
          {detail.neoRunId ? ` · Neo run ${detail.neoRunId}` : ''}
        </span>
      </div>
      {view === 'formatted' && <FormattedReply detail={detail} />}
      {view === 'raw' && <Block text={reply.raw ?? '(no body)'} />}
      {view === 'request' && <Block text={detail.request.raw} />}
    </div>
  )
}

/**
 * A closed row's one-line preview: a returned string decoded (its line breaks then show as spaces
 * in the one line), even when the preview cut it short; anything else as the model read it.
 */
export function previewText(preview: string): string {
  const first = preview.split('\n', 1)[0] ?? ''
  if (!first.startsWith('"')) return preview
  // The preview is the reply's first characters (`run-store.ts`), so it can end inside an escape
  // (`\`, `\u00`). Only a cut between escapes parses once closed with a quote, so the first of
  // these that parses is the decoded text up to the cut; an escape is at most 6 characters.
  const cuts = [0, 1, 2, 3, 4, 5].map(
    (drop) => `${first.slice(0, first.length - drop)}"`,
  )
  for (const candidate of [first, ...cuts]) {
    try {
      const value: unknown = JSON.parse(candidate)
      if (typeof value === 'string') return value
    } catch {
      // cut inside an escape, or not a string: try the next form
    }
  }
  return preview
}

/**
 * The reply for a person. A returned string (a snapshot, say) is shown decoded, with a line saying
 * so; "Reply, raw" keeps the exact bytes.
 */
export function FormattedReply({ detail }: { detail: RunDetail }) {
  const { reply } = detail
  const decoded = returnedString(detail)
  return (
    <div className="space-y-2">
      {reply.rpcError && <Block text={prettyJson(reply.rpcError)} />}
      {decoded ? (
        <>
          <p className="font-medium text-xs text-zinc-700">
            What the model read
          </p>
          <p className="text-xs text-zinc-500">{decodedNote(decoded.json)}</p>
          <Block text={decoded.value} />
          {decoded.rest && <Block text={decoded.rest} />}
        </>
      ) : (
        reply.text !== null && <Block text={reply.text} />
      )}
      {reply.structuredContent !== null && (
        <>
          <p className="text-xs text-zinc-500">structuredContent</p>
          <Block text={prettyJson(reply.structuredContent)} />
        </>
      )}
      {reply.notifications.length > 0 && (
        <>
          <p className="text-xs text-zinc-500">Streamed before the reply</p>
          <Block
            text={reply.notifications
              .map((message) => JSON.stringify(message))
              .join('\n')}
          />
        </>
      )}
    </div>
  )
}

function Block({ text }: { text: string }) {
  return (
    <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-all rounded bg-zinc-50 p-2 font-mono text-xs">
      {text}
    </pre>
  )
}
