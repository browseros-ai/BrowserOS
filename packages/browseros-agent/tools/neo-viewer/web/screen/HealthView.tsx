/**
 * The Health view: Neo's metrics in the main area, in place of the runs while it is open, so the
 * top bar's health, the time strip and the sidebar (live screen, the selected reply) keep
 * streaming beside it; a modal would make all of that inert. The top bar's health line opens it;
 * "← Runs" or Escape go back, and the runs come back as they were (the selection and the open
 * rows live on the screen, not here).
 *
 *   Runs        all runs: count (failed share in red), failed, p50, p95, histogram; then by status
 *   Operations  one row per op: calls, failed, p50, p95, histogram, most called first
 *   Errors      by owner and code, most first
 *   Tabs        open now and opened, by origin; closed, by reason, with their lifetimes
 *   Crashes     renderer crashes
 *   Raw         every metric family as Neo sent it, folded
 *
 * Every count here is Neo's own since it started (the top bar counts since the last Clear). The
 * sections share one column grid with the run table's type, hairlines and bars; a histogram is a
 * row of bucket columns on Neo's own bounds, so reading down a column shows which op is slow.
 * Numbers are read again on every health change (`useMetrics`) and the last ones stay until
 * new ones land, so nothing blanks while they move.
 */

import { type ReactNode, useState } from 'react'
import type {
  MetricsView,
  Quantiles,
  TelemetryHealth,
  ViewerInfo,
} from '../../shared/api'
import { useMetrics } from '../use-telemetry'
import { freshness } from './HealthLine'
import { share } from './measures'
import {
  axisEnds,
  bucketCounts,
  bucketsTitle,
  familyLines,
  quantileText,
  ranked,
} from './metrics'
import { LineSpans } from './ui'
import { count, plural } from './units'

/** The grid every section's rows share. */
const NAME = 'w-[196px] shrink-0 truncate pr-3'
const BARS = 'flex w-[170px] shrink-0 items-center gap-2'
const FAILED = 'w-[64px] shrink-0 text-right'
const QUANTILE = 'w-[72px] shrink-0 text-right'
const SPREAD = 'ml-6 w-24 shrink-0'

export function HealthView({
  health,
  info,
  onBack,
}: {
  health: TelemetryHealth | null
  info: ViewerInfo | null
  onBack: () => void
}) {
  const metrics = useMetrics(true, health?.rev)
  return (
    <section className="flex min-w-[752px] flex-col" aria-label="Neo health">
      <div className="sticky top-0 z-10">
        <div className="flex h-[30px] items-center gap-4 border-line border-b bg-panel px-4 text-[11px] leading-[14px]">
          <button
            type="button"
            onClick={onBack}
            title="Back to the runs (Escape)"
            className="w-[102px] shrink-0 text-left font-medium text-ink-2 hover:text-ink"
          >
            ← Runs
          </button>
          <h2 className="font-semibold text-ink">Neo health</h2>
          <span className="flex-1" />
          {health && metrics && (
            <span className="truncate text-ink-2">
              {freshness(health, Date.now()).where.replace(/\.$/, '')}
            </span>
          )}
        </div>
        <div className="flex h-6 items-center gap-[18px] overflow-hidden whitespace-nowrap border-line border-b bg-panel px-4 text-[11px] text-ink-2 leading-[14px]">
          <span>
            <b className="font-semibold text-ink">Since Neo started</b>: Neo's
            own counts; the top bar's are since the last Clear.
          </span>
          <span>
            <b className="font-semibold text-ink">p50</b>,{' '}
            <b className="font-semibold text-ink">p95</b>: half, and 95 in 100,
            took at most this long.
          </span>
        </div>
      </div>
      {metrics === undefined ? (
        <Note>Reading Neo's metrics…</Note>
      ) : metrics === null ? (
        <Note>
          No live metrics: Neo has sent none in the last 10 s. The viewer
          scrapes Neo's GET /v1/metrics every 2 s when it has a host API, and
          takes OTLP pushes at{' '}
          <span className="font-code text-ink">
            {info?.otlp.endpoint ?? '/otlp'}
          </span>
          .
        </Note>
      ) : (
        <Metrics metrics={metrics} />
      )}
    </section>
  )
}

function Note({ children }: { children: ReactNode }) {
  return (
    <p className="border-line-2 border-b px-4 py-6 text-ink-2 text-xs leading-4">
      {children}
    </p>
  )
}

function Metrics({ metrics }: { metrics: MetricsView }) {
  const { runs, operations, errors, tabs } = metrics
  const okRuns = runs.byStatus.ok ?? 0
  const failedRuns = Math.max(0, runs.total - okRuns)
  const opCalls = operations.reduce((sum, op) => sum + op.total, 0)
  const opErrors = operations.reduce((sum, op) => sum + op.errors, 0)
  const maxCalls = Math.max(0, ...operations.map((op) => op.total))
  const errorTotal = errors.reduce((sum, error) => sum + error.count, 0)
  const open = ranked(tabs.openByOrigin)
  const openNow = open.reduce((sum, [, n]) => sum + n, 0)
  const opened = ranked(tabs.openedByOrigin)
  const openedTotal = opened.reduce((sum, [, n]) => sum + n, 0)
  const origins = [
    ...new Set([...open.map(([key]) => key), ...opened.map(([key]) => key)]),
  ]
  const closed = ranked(tabs.closedByReason)
  const closedTotal = closed.reduce((sum, [, n]) => sum + n, 0)
  const statuses = ranked(runs.byStatus)
  return (
    <>
      <Section
        title="Runs"
        lines={[
          <>
            <Figure>{count(runs.total)}</Figure> ended
          </>,
          <>
            <Figure>{count(runs.inFlight)}</Figure> in flight now
          </>,
        ]}
      >
        <Columns
          name="Status"
          bars="Runs"
          failed="Not ok"
          quantiles
          spread={runs.duration}
        />
        <Row>
          <span className={`${NAME} font-plex text-ink`}>All runs</span>
          <Bars
            value={runs.total}
            failed={failedRuns}
            max={runs.total}
            title={`${plural(runs.total, 'run')} ended, ${count(failedRuns)} not ok`}
          />
          <Failed value={failedRuns} />
          <span className={QUANTILE}>{quantileText(runs.duration, 0.5)}</span>
          <span className={QUANTILE}>{quantileText(runs.duration, 0.95)}</span>
          <Spread quantiles={runs.duration} />
        </Row>
        {statuses.map(([status, n]) => (
          <Row key={status} sub>
            <span className={`${NAME} pl-3 text-ink-2`}>{status}</span>
            <Bars
              value={n}
              failed={status === 'ok' ? 0 : n}
              max={runs.total}
              title={`${plural(n, 'run')} ended ${status}`}
            />
          </Row>
        ))}
      </Section>
      <Section
        title="Operations"
        lines={[
          <>
            <Figure>{count(opCalls)}</Figure> calls,{' '}
            {plural(operations.length, 'method')}
          </>,
          <>
            <Figure bad={opErrors > 0}>{count(opErrors)}</Figure> failed
          </>,
        ]}
      >
        {operations.length === 0 ? (
          <Empty>No operation has settled since Neo started.</Empty>
        ) : (
          <>
            <Columns
              name="Method"
              bars="Calls"
              failed="Failed"
              quantiles
              spread={operations[0]?.duration}
            />
            {operations.map((op) => (
              <Row key={op.op}>
                <span className={`${NAME} text-ink`} title={op.op}>
                  {op.op}
                </span>
                <Bars
                  value={op.total}
                  failed={op.errors}
                  max={maxCalls}
                  title={`${plural(op.total, 'call')}, ${count(op.errors)} failed`}
                />
                <Failed value={op.errors} />
                <span className={QUANTILE}>
                  {quantileText(op.duration, 0.5)}
                </span>
                <span className={QUANTILE}>
                  {quantileText(op.duration, 0.95)}
                </span>
                <Spread quantiles={op.duration} />
              </Row>
            ))}
          </>
        )}
      </Section>
      <Section
        title="Errors"
        lines={[
          <>
            <Figure bad={errorTotal > 0}>{count(errorTotal)}</Figure> attributed
          </>,
          'each counted once',
        ]}
      >
        {errors.length === 0 ? (
          <Empty>No errors since Neo started.</Empty>
        ) : (
          <>
            <Columns name="Owner · code" bars="Count" />
            {errors.map((error) => (
              <Row key={`${error.owner}/${error.code}`}>
                <span className={NAME} title={`${error.owner} / ${error.code}`}>
                  <span className="font-plex text-[11px] text-ink-3">
                    {error.owner}
                  </span>{' '}
                  <span className="text-ink">{error.code}</span>
                </span>
                <Bars
                  value={error.count}
                  failed={error.count}
                  max={errors[0]?.count ?? error.count}
                  title={`${plural(error.count, 'error')}: ${error.owner} / ${error.code}`}
                />
              </Row>
            ))}
          </>
        )}
      </Section>
      <Section
        title="Tabs"
        lines={[
          <>
            <Figure>{count(openNow)}</Figure> open now
          </>,
          <>
            <Figure>{count(openedTotal)}</Figure> opened,{' '}
            <Figure>{count(closedTotal)}</Figure> closed
          </>,
        ]}
      >
        {origins.length === 0 && closed.length === 0 ? (
          <Empty>No tab has opened since Neo started.</Empty>
        ) : (
          <>
            <Columns name="Origin" bars="Open now" failed="Opened" />
            {origins.map((origin) => (
              <Row key={origin}>
                <span className={`${NAME} text-ink`}>{origin}</span>
                <Bars
                  value={tabs.openByOrigin[origin] ?? 0}
                  failed={0}
                  max={Math.max(openNow, 1)}
                  title={`${plural(tabs.openByOrigin[origin] ?? 0, 'tab')} open now`}
                />
                <span className={FAILED}>
                  {count(tabs.openedByOrigin[origin] ?? 0)}
                </span>
              </Row>
            ))}
            <div className="h-2" />
            <Columns
              name="Closed by"
              bars="Closed"
              quantiles
              spread={tabs.lifetime}
              spreadLabel="Lifetime"
            />
            <Row>
              <span className={`${NAME} font-plex text-ink`}>All closed</span>
              <Bars
                value={closedTotal}
                failed={0}
                max={Math.max(closedTotal, 1)}
                title={`${plural(closedTotal, 'tab')} closed`}
              />
              <span className={FAILED} />
              <span className={QUANTILE}>
                {quantileText(tabs.lifetime, 0.5)}
              </span>
              <span className={QUANTILE}>
                {quantileText(tabs.lifetime, 0.95)}
              </span>
              <Spread quantiles={tabs.lifetime} />
            </Row>
            {closed.map(([reason, n]) => (
              <Row key={reason} sub>
                <span className={`${NAME} pl-3 text-ink-2`}>{reason}</span>
                <Bars
                  value={n}
                  failed={reason === 'crash' ? n : 0}
                  max={Math.max(closedTotal, 1)}
                  title={`${plural(n, 'tab')} closed by ${reason}`}
                />
              </Row>
            ))}
          </>
        )}
      </Section>
      <Section
        title="Crashes"
        lines={[
          <>
            <Figure bad={metrics.crashes > 0}>{count(metrics.crashes)}</Figure>{' '}
            renderer {metrics.crashes === 1 ? 'crash' : 'crashes'}
          </>,
        ]}
      >
        <p className="pt-px text-ink-2 text-xs leading-4">
          {metrics.crashes === 0
            ? "No tab's page process has crashed since Neo started."
            : `${plural(metrics.crashes, "tab's page process", "tabs' page processes")} crashed since Neo started, each counted once, active or idle.`}
        </p>
      </Section>
      <RawFamilies metrics={metrics} />
    </>
  )
}

/** A section: its title and headline figures on the left, as the time strip has them; its rows. */
function Section({
  title,
  lines,
  children,
}: {
  title: string
  lines: ReactNode[]
  children: ReactNode
}) {
  return (
    <section
      aria-label={title}
      className="flex gap-5 border-line border-b px-4 pt-3 pb-2.5"
    >
      <div className="flex w-44 shrink-0 flex-col gap-[3px]">
        <h3 className="font-semibold text-[13px] text-black leading-4">
          {title}
        </h3>
        <p className="text-ink-2 text-xs leading-4">
          {lines.map((line, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: a summary line is its place.
            <span key={index} className="block">
              {line}
            </span>
          ))}
        </p>
      </div>
      <div className="flex min-w-0 flex-1 flex-col">{children}</div>
    </section>
  )
}

/** A table's column labels, in the run block's label type, over the shared grid. */
function Columns({
  name,
  bars,
  failed,
  quantiles = false,
  spread,
  spreadLabel = 'Duration',
}: {
  name: string
  bars: string
  failed?: string
  quantiles?: boolean
  spread?: Quantiles
  spreadLabel?: string
}) {
  const label = 'text-[10px] text-ink-3 uppercase leading-[14px] tracking-wide'
  const ends = spread ? axisEnds(spread) : null
  return (
    <div className="flex h-5 shrink-0 items-end pb-0.5">
      <span className={`${NAME} ${label}`}>{name}</span>
      <span className={`w-[170px] shrink-0 ${label}`}>{bars}</span>
      {(failed || quantiles) && (
        <span className={`${FAILED} ${label}`}>{failed}</span>
      )}
      {quantiles && (
        <>
          <span className={`${QUANTILE} ${label}`}>p50</span>
          <span className={`${QUANTILE} ${label}`}>p95</span>
        </>
      )}
      {ends && (
        <span
          className={`${SPREAD} flex justify-between font-code text-[10px] text-ink-3 leading-[14px]`}
          title={`${spreadLabel}: how many fell in each of Neo's buckets, ${ends.first} to ${ends.last} and over`}
        >
          <span>{ends.first}</span>
          <span>{ends.last}</span>
        </span>
      )}
    </div>
  )
}

function Row({
  children,
  sub = false,
}: {
  children: ReactNode
  sub?: boolean
}) {
  return (
    <div
      className={`flex h-7 shrink-0 items-center border-line-2 border-t font-code text-xs leading-4 ${sub ? 'text-ink-2' : 'text-black'}`}
    >
      {children}
    </div>
  )
}

/** What a section with nothing to list says, in place of its rows. */
function Empty({ children }: { children: ReactNode }) {
  return <p className="pt-px text-ink-2 text-xs leading-4">{children}</p>
}

/** A figure in a summary line, mono as the time strip's are. */
function Figure({
  children,
  bad = false,
}: {
  children: ReactNode
  bad?: boolean
}) {
  return (
    <span className={`font-code ${bad ? 'text-bad' : 'text-ink'}`}>
      {children}
    </span>
  )
}

/**
 * A count with its bar, as the run table's Neo time: the bar a share of the largest in its table,
 * its failed part in red at its end.
 */
function Bars({
  value,
  failed,
  max,
  title,
}: {
  value: number
  failed: number
  max: number
  title: string
}) {
  const ok = Math.max(0, value - failed)
  return (
    <span className={BARS} title={title}>
      <span className="flex h-2.5 w-[110px] shrink-0">
        {value > 0 && (
          <span
            data-bar="count"
            className="flex h-2.5"
            style={{ width: `max(2px, ${share(value, max) * 100}%)` }}
          >
            {ok > 0 && (
              <span
                className="block h-2.5 bg-ink"
                style={{ width: `${(ok / value) * 100}%` }}
              />
            )}
            {failed > 0 && (
              <span
                data-bar="failed"
                className="block h-2.5 bg-bad"
                style={{ width: `${(failed / value) * 100}%` }}
              />
            )}
          </span>
        )}
      </span>
      <span>{count(value)}</span>
    </span>
  )
}

/** A failed count: red when anything failed, a quiet zero otherwise. */
function Failed({ value }: { value: number }) {
  return (
    <span className={`${FAILED} ${value > 0 ? 'text-bad' : 'text-ink-4'}`}>
      {count(value)}
    </span>
  )
}

/**
 * A histogram at a glance: one column per bucket of Neo's own bounds, its height the bucket's
 * share of the busiest one; an empty bucket is a hairline, so the axis stays visible.
 */
function Spread({ quantiles }: { quantiles: Quantiles }) {
  const buckets = bucketCounts(quantiles)
  const peak = Math.max(0, ...buckets.map((bucket) => bucket.count))
  return (
    <span
      data-spread=""
      title={bucketsTitle(quantiles)}
      className={`${SPREAD} flex h-3.5 items-end gap-0.5`}
    >
      {buckets.map((bucket) => (
        <span
          key={bucket.le ?? 'inf'}
          className={`block min-w-0 flex-1 ${bucket.count > 0 ? 'bg-ink-2' : 'bg-line'}`}
          style={{
            height:
              bucket.count > 0
                ? `max(2px, ${share(bucket.count, peak) * 100}%)`
                : '1px',
          }}
        />
      ))}
    </span>
  )
}

/** Every family as Neo sent it, folded under one quiet row as the run block folds Neo's own. */
function RawFamilies({ metrics }: { metrics: MetricsView }) {
  const [open, setOpen] = useState(false)
  const families = metrics.families
  return (
    <Section
      title="Raw"
      lines={[
        <>
          <Figure>{count(families.length)}</Figure> metric{' '}
          {families.length === 1 ? 'family' : 'families'}
        </>,
        metrics.source === 'scrape' ? 'as scraped' : 'as pushed',
      ]}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex h-6 shrink-0 items-center gap-2 text-left text-[11px] text-ink-3 leading-[14px] hover:text-ink"
      >
        <span>{open ? '▾' : '▸'}</span>
        <span>
          {open ? 'Hide' : 'Show'} every family, in Prometheus' text form:{' '}
          {families.map((family) => family.name).join(', ')}
        </span>
      </button>
      {open && (
        <div className="mt-1 mb-1 max-h-[480px] overflow-auto rounded-[5px] bg-panel">
          <pre className="w-max min-w-full px-3 py-2.5 font-code text-[11px] text-ink leading-[15px]">
            {familyLines(families).map((line, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: an output line is its line number.
              <div key={index}>
                <LineSpans line={line} />
                {line.length === 0 && ' '}
              </div>
            ))}
          </pre>
        </div>
      )}
    </Section>
  )
}
