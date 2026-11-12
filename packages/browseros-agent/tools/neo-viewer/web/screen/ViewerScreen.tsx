/**
 * Viewer C v2, the designed main screen (Paper file "neo · trace viewer", artboards "Viewer C v2:
 * …"). It reads the same API as the plain page through `useViewer`, which keeps every run since
 * the last Clear current from `/api/events`: a run appears when its call reaches the proxy and
 * fills in when Neo's reply passes back. Every agent's runs share one list (owner, 2026-11-11).
 *
 *   ┌ TopBar: name · container line · MCP URL + Copy · Export · Clear · Take over ─┐
 *   ├ ExportNotice: the last export's folder + Copy path, until dismissed ─────────┤
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
 * shows up there as it arrives; clicking a row opens it and pins the sidebar to it. Inside an
 * open run, clicking a call selects it, and the sidebar shows what that call returned until the
 * run's "Model read" row, another run, or Clear selects a reply again. "Open full" opens
 * whichever the sidebar shows.
 */

import { useEffect, useState } from 'react'
import type { ExportResponse, RunSummary } from '../../shared/api'
import { LiveScreenPopup } from '../live-screen/LiveScreen'
import { useLiveScreen } from '../live-screen/use-live-screen'
import { useViewer } from '../use-viewer'
import { FullOutput } from './FullOutput'
import { runTotals } from './measures'
import type { OperationSelection } from './operations'
import { RunTable } from './RunTable'
import { Sidebar } from './Sidebar'
import { Splitter, useSidebarWidth } from './Splitter'
import { DEFAULT_SORT, type RunSort } from './sort'
import { TimeStrip } from './TimeStrip'
import { ExportNotice, TopBar } from './TopBar'
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
  const { info, runs, log, clear, exportRuns, error } = useViewer()
  const [exported, setExported] = useState<ExportResponse | null>(null)
  const [exporting, setExporting] = useState(false)
  const [chosenId, setChosenId] = useState<string | null>(null)
  const [openIds, setOpenIds] = useState<ReadonlySet<string>>(new Set())
  const [sort, setSort] = useState<RunSort>(DEFAULT_SORT)
  const [fullId, setFullId] = useState<string | null>(null)
  const [operation, setOperation] = useState<OperationSelection | null>(null)
  const [fullOperation, setFullOperation] = useState<OperationSelection | null>(
    null,
  )
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
  // A call is shown only while its run is the selected one.
  const selectedOperation =
    operation && operation.runId === selected?.id ? operation : null

  const onClear = () => {
    setChosenId(null)
    setOpenIds(new Set())
    setFullId(null)
    setOperation(null)
    setFullOperation(null)
    void clear()
  }

  const onExport = () => {
    setExporting(true)
    void exportRuns()
      .then((written) => {
        if (written) setExported(written)
      })
      .finally(() => setExporting(false))
  }

  const onRow = (run: RunSummary) => {
    const open = openIds.has(run.id)
    const next = new Set(openIds)
    if (open && run.id === selected?.id) {
      next.delete(run.id)
    } else {
      next.add(run.id)
      if (run.id !== selected?.id) setOperation(null)
      setChosenId(run.id)
    }
    setOpenIds(next)
  }

  const onSelectOperation = (
    run: RunSummary,
    selection: OperationSelection | null,
  ) => {
    setChosenId(run.id)
    setOperation(selection)
  }

  const openFull = (run: RunSummary) => {
    setChosenId(run.id)
    setFullId(run.id)
    setFullOperation(null)
  }

  const openFullOperation = (selection: OperationSelection) => {
    setChosenId(selection.runId)
    setFullId(selection.runId)
    setFullOperation(selection)
  }

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-white font-plex text-ink text-xs leading-4 antialiased">
      <TopBar
        info={info && { ...info, container }}
        onClear={onClear}
        onExport={onExport}
        exporting={exporting}
        live={live}
      />
      {exported && (
        <ExportNotice exported={exported} onDismiss={() => setExported(null)} />
      )}
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
            selectedOperation={selectedOperation}
            onRow={onRow}
            onSelectOperation={onSelectOperation}
            onOpenFull={openFull}
            nowMs={nowMs}
          />
        </main>
        <Splitter width={sidebarWidth} onWidth={setSidebarWidth} />
        <Sidebar
          width={sidebarWidth}
          run={selected}
          operation={selectedOperation}
          onShowReply={() => setOperation(null)}
          onOpenFull={openFull}
          onOpenFullOperation={openFullOperation}
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
          operation={fullOperation?.runId === fullRun.id ? fullOperation : null}
          onOperation={(selection) => {
            setFullOperation(selection)
            setOperation(selection)
          }}
          onClose={() => {
            setFullId(null)
            setFullOperation(null)
          }}
        />
      )}
    </div>
  )
}
