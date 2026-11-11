/**
 * Viewer C v2, the designed main screen (Paper file "neo · trace viewer", artboards "Viewer C v2:
 * …"). It reads the same API as the plain page through `useViewer`, which keeps every run since
 * the last Clear current from `/api/events`: a run appears when its call reaches the proxy and
 * fills in when Neo's reply passes back. Every agent's runs share one list (owner, 2026-11-11).
 *
 *   ┌ TopBar: name · container line · MCP URL + Copy · Clear · Take over ──────────┐
 *   ├ TimeStrip: where the wall clock went, since the last Clear ──────────────────┤
 *   │ RunTable: one row per run(); open rows show RunBlock    ┃ Sidebar: live      │
 *   │ (code + "Model read"); totals row                       ┃ screen, then the   │
 *   │                                                Splitter ┃ selected reply     │
 *   └────────────────────────────────────────────────────────────────────────────────┘
 *   FullOutput: the pop-up "Open full" opens over all of it.
 *   LiveScreenPopup: "Pop out" moves the live screen into a large pop-up; closing docks it again.
 *
 * The live screen is PR C2's (`web/live-screen/`): one controller (`useLiveScreen`) drives the
 * docked panel, the pop-up and Take over, so they always agree on the mode and the takeover.
 * It follows the agent: the tab its latest run used, the one Take over takes (`LiveStatus.tab`).
 *
 * Selection: until a row is clicked, the sidebar follows the latest run, so a new run
 * shows up there as it arrives; clicking a row opens it and pins the sidebar to it.
 */

import { useEffect, useState } from 'react'
import type { RunSummary } from '../../shared/api'
import { LiveScreenPopup } from '../live-screen/LiveScreen'
import { useLiveScreen } from '../live-screen/use-live-screen'
import { useViewer } from '../use-viewer'
import { FullOutput } from './FullOutput'
import { runTotals } from './measures'
import { RunTable } from './RunTable'
import { Sidebar } from './Sidebar'
import { Splitter, useSidebarWidth } from './Splitter'
import { DEFAULT_SORT, type RunSort } from './sort'
import { TimeStrip } from './TimeStrip'
import { TopBar } from './TopBar'
import { useContainer } from './use-container'

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
  const { info, runs, log, clear, error } = useViewer()
  const [chosenId, setChosenId] = useState<string | null>(null)
  const [openIds, setOpenIds] = useState<ReadonlySet<string>>(new Set())
  const [sort, setSort] = useState<RunSort>(DEFAULT_SORT)
  const [fullId, setFullId] = useState<string | null>(null)
  const [sidebarWidth, setSidebarWidth] = useSidebarWidth()
  const live = useLiveScreen()
  const container = useContainer(info)
  const [popped, setPopped] = useState(false)
  const nowMs = useNow(runs.some((run) => run.status === 'running'))

  // `runs` is in call order; with no row chosen the sidebar follows the newest.
  const selected =
    runs.find((run) => run.id === chosenId) ?? runs.at(-1) ?? null
  const fullRun = runs.find((run) => run.id === fullId) ?? null
  const totals = runTotals(runs)

  const onClear = () => {
    setChosenId(null)
    setOpenIds(new Set())
    setFullId(null)
    void clear()
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
      <TopBar
        info={info && { ...info, container }}
        onClear={onClear}
        live={live}
      />
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
          <RunTable
            runs={runs}
            totals={totals}
            toolsList={log?.toolsList ?? null}
            sort={sort}
            onSort={setSort}
            openIds={openIds}
            selectedId={selected?.id ?? null}
            onRow={onRow}
            onOpenFull={openFull}
            nowMs={nowMs}
          />
        </main>
        <Splitter width={sidebarWidth} onWidth={setSidebarWidth} />
        <Sidebar
          width={sidebarWidth}
          run={selected}
          onOpenFull={openFull}
          live={live}
          popped={popped}
          onPopOut={() => setPopped(true)}
        />
      </div>
      {popped && (
        <LiveScreenPopup live={live} onClose={() => setPopped(false)} />
      )}
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
