/**
 * The plain run list (`?plain`) with the live screen beside it: every run since the last Clear,
 * in order, whichever agent sent it; watch or take over the agent's tab. The designed Viewer C v2
 * screen reads the same API and uses the same live-screen components.
 */

import { useState } from 'react'
import type { LogSummary } from '../shared/api'
import { RunList } from './components/RunList'
import { duration, tokens } from './format'
import {
  LiveScreen,
  LiveScreenPopup,
  TakeOverButton,
} from './live-screen/LiveScreen'
import { useLiveScreen } from './live-screen/use-live-screen'
import { useViewer } from './use-viewer'

export function App() {
  const { info, runs, log, clear, error } = useViewer()
  // Follows the agent's latest run: its tab, or the task it named.
  const live = useLiveScreen()
  const [popped, setPopped] = useState(false)
  return (
    <div className="mx-auto flex max-w-6xl items-start gap-6 p-6 font-sans text-sm text-zinc-900">
      <div className="min-w-0 flex-1 space-y-4">
        <header className="space-y-1">
          <div className="flex items-center gap-3">
            <h1 className="font-semibold text-lg">Neo viewer</h1>
            <div className="flex-1" />
            <button
              type="button"
              className="rounded border border-zinc-300 px-2 py-0.5 hover:bg-zinc-50"
              title="Empty the list. The previous log is kept once, as log.prev.jsonl."
              onClick={() => void clear()}
            >
              Clear
            </button>
            <TakeOverButton live={live} />
          </div>
          {info && <McpUrl url={info.mcpUrl} upstream={info.upstreamUrl} />}
          {error && <p className="text-red-700">{error}</p>}
        </header>
        {log && <LogHeader log={log} />}
        {runs.length === 0 ? (
          <p className="text-zinc-600">
            No runs yet. Give an agent the MCP URL above; every run() call any
            agent makes appears here, in order.
          </p>
        ) : (
          <RunList runs={runs} />
        )}
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

function LogHeader({ log }: { log: LogSummary }) {
  const { totals } = log
  return (
    <section className="space-y-1 rounded border border-zinc-200 p-3">
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
        <span>tools/list {tokens(log.toolsList)}</span>
        <span className="text-zinc-500">
          since {new Date(log.clearedAt).toLocaleTimeString()}
        </span>
      </p>
    </section>
  )
}
