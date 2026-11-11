/**
 * The plain run list (PR B) with the live screen beside it: pick a task, read its runs in
 * order, watch or take over its tab. The designed Viewer C v2 screen replaces this page (PR D)
 * on the same API and the same live-screen components.
 */

import { useState } from 'react'
import type { TaskSummary } from '../shared/api'
import { RunList } from './components/RunList'
import { duration, identification, tokens } from './format'
import {
  LiveScreen,
  LiveScreenPopup,
  TakeOverButton,
} from './live-screen/LiveScreen'
import { useLiveScreen } from './live-screen/use-live-screen'
import { useViewer } from './use-viewer'

export function App() {
  const { info, tasks, taskId, selectTask, runs, error } = useViewer()
  const task = tasks.find((candidate) => candidate.id === taskId) ?? null
  const live = useLiveScreen(task?.neoTaskId ?? null)
  const [popped, setPopped] = useState(false)
  return (
    <div className="mx-auto flex max-w-6xl items-start gap-6 p-6 font-sans text-sm text-zinc-900">
      <div className="min-w-0 flex-1 space-y-4">
        <header className="space-y-1">
          <div className="flex items-center gap-3">
            <h1 className="font-semibold text-lg">Neo viewer</h1>
            <div className="flex-1" />
            <TakeOverButton live={live} />
          </div>
          {info && <McpUrl url={info.mcpUrl} upstream={info.upstreamUrl} />}
          {error && <p className="text-red-700">{error}</p>}
        </header>
        {tasks.length === 0 ? (
          <p className="text-zinc-600">
            No tasks yet. Give an agent the MCP URL above; each agent becomes a
            task.
          </p>
        ) : (
          <TaskPicker tasks={tasks} taskId={taskId} onSelect={selectTask} />
        )}
        {task && <TaskHeader task={task} />}
        {task && <RunList runs={runs} />}
      </div>
      <aside className="sticky top-6 w-[340px] shrink-0">
        <LiveScreen
          live={live}
          size="docked"
          popped={popped}
          onPopOut={() => setPopped(true)}
        />
      </aside>
      {popped && (
        <LiveScreenPopup live={live} onClose={() => setPopped(false)} />
      )}
    </div>
  )
}

function McpUrl({ url, upstream }: { url: string; upstream: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <p className="flex flex-wrap items-center gap-2 text-zinc-600">
      MCP URL{' '}
      <code className="rounded bg-zinc-100 px-1.5 py-0.5 font-mono text-zinc-900">
        {url}
      </code>
      <button
        type="button"
        className="rounded border border-zinc-300 px-2 py-0.5 hover:bg-zinc-50"
        onClick={() => {
          void navigator.clipboard.writeText(url).then(() => setCopied(true))
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
      <span>forwards to {upstream}</span>
    </p>
  )
}

function TaskPicker({
  tasks,
  taskId,
  onSelect,
}: {
  tasks: TaskSummary[]
  taskId: string | null
  onSelect: (taskId: string) => void
}) {
  return (
    <label className="flex items-center gap-2">
      Task
      <select
        className="rounded border border-zinc-300 px-2 py-1"
        value={taskId ?? ''}
        onChange={(event) => onSelect(event.target.value)}
      >
        {tasks.map((task) => (
          <option key={task.id} value={task.id}>
            {task.name} · {task.totals.runs} runs
            {task.identifiedBy.kind === 'agent-task-id'
              ? ` · ${task.identifiedBy.taskId}`
              : ''}
          </option>
        ))}
      </select>
    </label>
  )
}

function TaskHeader({ task }: { task: TaskSummary }) {
  const { totals } = task
  return (
    <section className="space-y-1 rounded border border-zinc-200 p-3">
      <p>
        <span className="font-medium">{task.name}</span>{' '}
        <span className="text-zinc-600">
          · {task.client.title ?? task.client.name}
          {task.client.version ? ` ${task.client.version}` : ''} · grouped by{' '}
          {identification(task.identifiedBy)} · Neo taskId{' '}
          <code className="font-mono">{task.neoTaskId}</code>
        </span>
      </p>
      <p className="flex flex-wrap gap-x-4 text-zinc-700">
        <span>{totals.runs} runs</span>
        {totals.running > 0 && <span>{totals.running} running</span>}
        {totals.failed > 0 && (
          <span className="text-red-700">{totals.failed} failed</span>
        )}
        <span>round trips {duration(totals.roundTripMs)}</span>
        <span>between calls {duration(totals.gapMs)}</span>
        <span>wall clock {duration(totals.wallClockMs)}</span>
        <span>code {tokens(totals.code)}</span>
        <span>replies {tokens(totals.reply)}</span>
        <span>tools/list {tokens(task.toolsList)}</span>
      </p>
    </section>
  )
}
