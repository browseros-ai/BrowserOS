/**
 * Viewer C v2, the designed main screen (Paper file "neo · trace viewer", artboards "Viewer C v2:
 * …"). It reads the same API as the plain page through `useViewer`, which keeps the task list
 * and the chosen task's runs current from `/api/events`: a run appears when its call reaches the
 * proxy and fills in when Neo's reply passes back.
 *
 *   ┌ TopBar: name · container line · MCP URL + Copy · Task menu ──────────────────┐
 *   ├ TimeStrip: where the task's wall clock went ─────────────────────────────────┤
 *   │ RunTable: one row per run(); open rows show RunBlock    ┃ Sidebar: live      │
 *   │ (code + "Model read"); totals row                       ┃ screen, then the   │
 *   │                                                Splitter ┃ selected reply     │
 *   └────────────────────────────────────────────────────────────────────────────────┘
 *   FullOutput: the pop-up "Open full" opens over all of it.
 *
 * Selection: until a row is clicked, the sidebar follows the task's latest run, so a new run
 * shows up there as it arrives; clicking a row opens it and pins the sidebar to it.
 */

import { useEffect, useState } from 'react'
import type { RunSummary } from '../../shared/api'
import { useViewer } from '../use-viewer'
import { FullOutput } from './FullOutput'
import { runTotals } from './measures'
import { RunTable } from './RunTable'
import { Sidebar } from './Sidebar'
import { Splitter, useSidebarWidth } from './Splitter'
import { DEFAULT_SORT, type RunSort } from './sort'
import { TimeStrip } from './TimeStrip'
import { TopBar } from './TopBar'

/** How often a running run's elapsed time and the live strip redraw. */
const TICK_MS = 250

function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!ticking) return
    setNow(Date.now())
    const timer = setInterval(() => setNow(Date.now()), TICK_MS)
    return () => clearInterval(timer)
  }, [ticking])
  return now
}

export function ViewerScreen() {
  const { info, tasks, taskId, selectTask, runs, error } = useViewer()
  const task = tasks.find((candidate) => candidate.id === taskId) ?? null
  const [chosenId, setChosenId] = useState<string | null>(null)
  const [openIds, setOpenIds] = useState<ReadonlySet<string>>(new Set())
  const [sort, setSort] = useState<RunSort>(DEFAULT_SORT)
  const [fullId, setFullId] = useState<string | null>(null)
  const [sidebarWidth, setSidebarWidth] = useSidebarWidth()
  const nowMs = useNow(runs.some((run) => run.status === 'running'))

  // `runs` is in call order; with no row chosen the sidebar follows the newest.
  const selected =
    runs.find((run) => run.id === chosenId) ?? runs.at(-1) ?? null
  const fullRun = runs.find((run) => run.id === fullId) ?? null
  const totals = runTotals(runs)

  const switchTask = (next: string) => {
    if (next === taskId) return
    setChosenId(null)
    setOpenIds(new Set())
    setFullId(null)
    selectTask(next)
  }

  const onRow = (run: RunSummary) => {
    const open = openIds.has(run.id)
    const next = new Set(openIds)
    if (open && run.id === selected?.id) {
      next.delete(run.id)
    } else {
      next.add(run.id)
      setChosenId(run.id)
    }
    setOpenIds(next)
  }

  const openFull = (run: RunSummary) => {
    setChosenId(run.id)
    setFullId(run.id)
  }

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-white font-plex text-ink text-xs leading-4 antialiased">
      <TopBar info={info} tasks={tasks} task={task} onTask={switchTask} />
      {error && (
        <p
          role="alert"
          className="shrink-0 border-line border-b bg-[#FDF0F0] px-5 py-1.5 text-bad"
        >
          {error}
        </p>
      )}
      <TimeStrip
        runs={runs}
        totals={totals}
        nowMs={nowMs}
        selectedId={selected?.id ?? null}
      />
      <div className="flex min-h-0 flex-1">
        <main className="min-w-0 flex-1 overflow-auto">
          {task ? (
            <RunTable
              runs={runs}
              totals={totals}
              toolsList={task.toolsList}
              sort={sort}
              onSort={setSort}
              openIds={openIds}
              selectedId={selected?.id ?? null}
              onRow={onRow}
              onOpenFull={openFull}
              nowMs={nowMs}
            />
          ) : (
            <NoTasks mcpUrl={info?.mcpUrl ?? null} />
          )}
        </main>
        <Splitter width={sidebarWidth} onWidth={setSidebarWidth} />
        <Sidebar width={sidebarWidth} run={selected} onOpenFull={openFull} />
      </div>
      {fullRun && (
        <FullOutput
          run={fullRun}
          runs={runs}
          onRun={openFull}
          onClose={() => setFullId(null)}
        />
      )}
    </div>
  )
}

function NoTasks({ mcpUrl }: { mcpUrl: string | null }) {
  return (
    <div className="flex max-w-xl flex-col gap-2 px-5 py-8">
      <h2 className="font-semibold text-[15px] text-ink leading-5">
        No tasks yet
      </h2>
      <p className="text-[13px] text-ink-2 leading-5">
        Give an agent the MCP URL above
        {mcpUrl && (
          <>
            {' '}
            (<code className="font-code text-ink">{mcpUrl}</code>)
          </>
        )}
        . Each agent connection becomes a task, named from its client and the
        time of its first call, and its run() calls appear here as they happen.
      </p>
    </div>
  )
}
