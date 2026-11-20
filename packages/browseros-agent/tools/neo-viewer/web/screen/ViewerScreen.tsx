/**
 * Viewer C v2, the designed main screen (Paper file "neo · trace viewer", artboards "Viewer C v2:
 * …"). It reads the same API as the plain page through `useViewer`, which keeps every run since
 * the last Clear current from `/api/events`: a run appears when its call reaches the proxy and
 * fills in when Neo's reply passes back. Every agent's runs share one list (owner, 2026-11-11).
 *
 *   ┌ TopBar: name · container line · health · MCP URL + Copy · Export · Clear · Take over ┐
 *   ├ ExportNotice: the last export's folder + Copy path, until dismissed ─────────────────┤
 *   ├ TimeStrip: where the wall clock went, since the last Clear ──────────────────────────┤
 *   │ RunTable: one row per run(); open rows show RunBlock    ┃ Sidebar: live              │
 *   │ (code + "Model read" + "Trace"); totals row             ┃ screen, then the selected  │
 *   │   or HealthView, in its place while open       Splitter ┃ reply, call or trace       │
 *   └────────────────────────────────────────────────────────────────────────────────────────┘
 *   FullOutput: the pop-up "Open full" opens over all of it.
 *   LiveScreenPopup: "Pop out" moves the live screen into a large pop-up; closing docks it again.
 *
 * Neo's telemetry adds to this only where it exists (`useViewer().health` is null for an image
 * older than telemetry, and then none of it shows): the top bar's health line, which opens the
 * Health view in the runs' place (the top bar, strip and sidebar keep streaming beside it, and
 * the runs keep their scroll, selection and open rows), and a run's trace (below).
 *
 * The live screen is PR C2's (`web/live-screen/`): one controller (`useLiveScreen`) drives the
 * docked panel, the pop-up and Take over, so they always agree on the mode and the takeover.
 * It follows the agent: the tab its latest run used, the one Take over takes (`LiveStatus.tab`).
 *
 * Selection: until a row is clicked, the sidebar follows the latest run, so a new run
 * shows up there as it arrives; clicking a row opens it and pins the sidebar to it. Inside an
 * open run, clicking a call selects it, and the sidebar shows what that call returned until the
 * run's "Model read" row, another run, or Clear selects a reply again. "Open full" opens
 * whichever the sidebar shows. A run's "Trace" row shows its OpenTelemetry waterfall there
 * instead; while it shows, a call selected in the block marks its span, and a span selected
 * in the waterfall opens its call in the block (one id, `neo.op.id`, names both).
 */

import { useCallback, useEffect, useState } from 'react'
import type { ExportResponse, RunSummary, TraceSpan } from '../../shared/api'
import { LiveScreenPopup } from '../live-screen/LiveScreen'
import { useLiveScreen } from '../live-screen/use-live-screen'
import { useRunTrace } from '../use-telemetry'
import { useRunDetail, useViewer } from '../use-viewer'
import { FullOutput } from './FullOutput'
import { HealthView } from './HealthView'
import { runTotals } from './measures'
import type { OperationSelection } from './operations'
import { RunTable } from './RunTable'
import { Sidebar } from './Sidebar'
import { Splitter, useSidebarWidth } from './Splitter'
import { DEFAULT_SORT, type RunSort } from './sort'
import { TimeStrip } from './TimeStrip'
import { ExportNotice, TopBar } from './TopBar'
import type { TraceSelection } from './TracePanel'
import { operationFor, selectedSpan } from './trace'
import { useContainer } from './use-container'
import { TraceFull } from './Waterfall'

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
  const { info, runs, log, health, clear, exportRuns, error } = useViewer()
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
  const [healthOpen, setHealthOpen] = useState(false)
  // The view needs the health line that opens it: none without telemetry.
  const healthShown = healthOpen && health !== null
  const closeHealth = useCallback(() => setHealthOpen(false), [])
  useEscapeCloses(healthShown, closeHealth)
  const [trace, setTrace] = useState<TraceSelection | null>(null)
  const [fullTraceId, setFullTraceId] = useState<string | null>(null)
  // Runs have traces once any span reached the viewer: from then on every open block has a row.
  const traceRev = health?.spansSeen ? health.rev : null

  // `runs` is in call order; with no row chosen the sidebar follows the newest.
  const selected =
    runs.find((run) => run.id === chosenId) ?? runs.at(-1) ?? null
  const fullRun = runs.find((run) => run.id === fullId) ?? null
  const totals = runTotals(runs)
  // A call is shown only while its run is the selected one; so is a trace.
  const selectedOperation =
    operation && operation.runId === selected?.id ? operation : null
  const selectedTrace = trace && trace.runId === selected?.id ? trace : null
  const fullTraceRun = runs.find((run) => run.id === fullTraceId) ?? null

  const onClear = () => {
    setChosenId(null)
    setOpenIds(new Set())
    setFullId(null)
    setOperation(null)
    setFullOperation(null)
    setTrace(null)
    setFullTraceId(null)
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
      if (run.id !== selected?.id) {
        setOperation(null)
        setTrace(null)
      }
      setChosenId(run.id)
    }
    setOpenIds(next)
  }

  // While the run's trace shows, a call picked in the block marks its span there (the span
  // follows the call); another run's call leaves the trace.
  const onSelectOperation = (
    run: RunSummary,
    selection: OperationSelection | null,
  ) => {
    setChosenId(run.id)
    setOperation(selection)
    setTrace((current) =>
      current?.runId === run.id ? { runId: run.id, spanId: null } : null,
    )
  }

  const showReply = (run: RunSummary) => {
    setChosenId(run.id)
    setOperation(null)
    setTrace(null)
  }

  const selectTrace = (run: RunSummary) => {
    setChosenId(run.id)
    setTrace((current) =>
      current?.runId === run.id ? current : { runId: run.id, spanId: null },
    )
  }

  // A span picked in the waterfall opens its call in the block, when the run's record has it.
  const selectSpan = (
    run: RunSummary,
    span: TraceSpan,
    call: OperationSelection | null,
  ) => {
    setChosenId(run.id)
    setTrace({ runId: run.id, spanId: span.spanId })
    setOperation(call)
  }

  const openFullTrace = (run: RunSummary) => {
    selectTrace(run)
    setFullTraceId(run.id)
  }

  // A reply in the pop-up is the selection too: the panel shows that reply, not an older call.
  const openFull = (run: RunSummary) => {
    setChosenId(run.id)
    setFullId(run.id)
    setFullOperation(null)
    setOperation(null)
    setTrace(null)
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
        health={health}
        healthOpen={healthShown}
        onHealth={() => setHealthOpen(!healthShown)}
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
        {/* The runs and the Health view scroll on their own, so each keeps its place. */}
        <main className="relative min-w-0 flex-1">
          <div
            className={`absolute inset-0 overflow-auto ${healthShown ? 'invisible' : ''}`}
          >
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
              traceRev={traceRev}
              traceRunId={selectedTrace?.runId ?? null}
              onShowReply={showReply}
              onSelectTrace={selectTrace}
              onOpenFullTrace={openFullTrace}
            />
          </div>
          {healthShown && (
            <div className="absolute inset-0 overflow-auto bg-white">
              <HealthView health={health} info={info} onBack={closeHealth} />
            </div>
          )}
        </main>
        <Splitter width={sidebarWidth} onWidth={setSidebarWidth} />
        <Sidebar
          width={sidebarWidth}
          run={selected}
          operation={selectedOperation}
          trace={selectedTrace}
          traceRev={health?.rev}
          onShowReply={() => {
            setOperation(null)
            setTrace(null)
          }}
          onOpenFull={openFull}
          onOpenFullOperation={openFullOperation}
          onSelectSpan={(span, call) =>
            selected && selectSpan(selected, span, call)
          }
          onShowCall={() => setTrace(null)}
          onOpenFullTrace={() => selected && openFullTrace(selected)}
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
      {fullTraceRun && (
        <FullTrace
          run={fullTraceRun}
          rev={health?.rev}
          spanId={trace?.runId === fullTraceRun.id ? trace.spanId : null}
          operationId={
            operation?.runId === fullTraceRun.id ? operation.operationId : null
          }
          onSelectSpan={(span, call) => selectSpan(fullTraceRun, span, call)}
          onClose={() => setFullTraceId(null)}
        />
      )}
    </div>
  )
}

/** "Open full" on a trace: the pop-up, once the run's trace has been read. */
function FullTrace({
  run,
  rev,
  spanId,
  operationId,
  onSelectSpan,
  onClose,
}: {
  run: RunSummary
  rev: number | undefined
  spanId: string | null
  operationId: string | null
  onSelectSpan: (span: TraceSpan, call: OperationSelection | null) => void
  onClose: () => void
}) {
  const trace = useRunTrace(run, true, rev)
  const fetched = useRunDetail(run, true)
  const record = fetched?.id === run.id ? fetched.neo : null
  if (!trace) return null
  const span = selectedSpan(trace, spanId, operationId)
  return (
    <TraceFull
      run={run}
      trace={trace}
      selectedSpanId={span?.spanId ?? null}
      onSelectSpan={(picked) =>
        onSelectSpan(picked, operationFor(run, record, picked))
      }
      onClose={onClose}
    />
  )
}

/**
 * Escape closes the Health view, unless it belongs to something else: a pop-up (its own Escape),
 * a field, or the live screen in Control, whose keys go to Neo's browser.
 */
function useEscapeCloses(active: boolean, close: () => void): void {
  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return
      const target = event.target as Element | null
      if (document.querySelector('dialog[open]')) return
      if (target?.closest?.('input, textarea, [data-display]')) return
      close()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, close])
}
