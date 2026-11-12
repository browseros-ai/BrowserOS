import { useState } from 'react'
import type { RunDetail, RunStatus, RunSummary } from '../../shared/api'
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
              {run.replyPreview}
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

function FormattedReply({ detail }: { detail: RunDetail }) {
  const { reply } = detail
  return (
    <div className="space-y-2">
      {reply.rpcError && <Block text={prettyJson(reply.rpcError)} />}
      {reply.text !== null && <Block text={reply.text} />}
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
