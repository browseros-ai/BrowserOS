/**
 * The full-width Metrics tab shows Neo's instance counters and measured operation durations.
 * It shares Health's metrics subscription, while each slow bucket hands its exact method and
 * duration interval to Traces. Histogram counts never inherit method-level failure totals.
 */
import type { ReactNode } from 'react'
import type { MetricsView, TelemetryHealth } from '../../shared/api'
import { useMetrics } from '../use-telemetry'
import type { ExplorerSpanFilter } from './explorer-filter'
import {
  DURATION_BANDS,
  failedDurationBandCount,
  operationBandCount,
  operationOutcomes,
  slowOperationBuckets,
} from './metrics-explorer'
import { clock, count, plural } from './units'

export function MetricsExplorer({
  health,
  onFindTraces,
}: {
  health: TelemetryHealth | null
  onFindTraces: (filter: ExplorerSpanFilter) => void
}) {
  const metrics = useMetrics(true, health?.rev)
  return (
    <section
      aria-label="Metrics"
      className="mx-auto flex w-full max-w-[1600px] flex-col px-6 pb-10 text-ink sm:px-[46px]"
    >
      <header className="flex flex-wrap items-center justify-between gap-4 pt-[22px] pb-[18px]">
        <div>
          <h1 className="font-semibold text-[30px] leading-[41px]">Metrics</h1>
          <p className="text-[15px] text-ink-2 leading-[21px]">
            Operation counts, outcomes and latency in this Neo instance.
          </p>
        </div>
        <div className="text-right text-[13px] text-ink-2 leading-[18px]">
          <p>Since Neo started</p>
          {metrics && (
            <p className="text-xs">
              {metrics.source === 'scrape' ? 'Host API scrape' : 'OTLP push'} ·
              sample {clock(metrics.observedAt)}
            </p>
          )}
        </div>
      </header>
      {metrics === undefined ? (
        <Notice>Reading Neo's metrics…</Notice>
      ) : metrics === null ? (
        <Notice>
          No live metrics. Neo has supplied no recent metric sample. Counts will
          appear when the viewer receives a host API scrape or an OTLP push.
        </Notice>
      ) : (
        <MetricsContents metrics={metrics} onFindTraces={onFindTraces} />
      )}
    </section>
  )
}

function MetricsContents({
  metrics,
  onFindTraces,
}: {
  metrics: MetricsView
  onFindTraces: (filter: ExplorerSpanFilter) => void
}) {
  const { operations, runs } = metrics
  const calls = operations.reduce(
    (total, operation) => total + operation.total,
    0,
  )
  const failed = operations.reduce((total, operation) => {
    const outcomes = operationOutcomes(metrics.families, operation)
    return (
      total + (outcomes.kind === 'labeled' ? outcomes.error : outcomes.failed)
    )
  }, 0)
  const measured = operations.reduce(
    (total, operation) => total + operation.duration.count,
    0,
  )
  const slow = operationBandCount(
    operations,
    { minMs: 500, maxMs: null },
    metrics.families,
  )
  const buckets = DURATION_BANDS.map((band) => ({
    ...band,
    count: operationBandCount(operations, band, metrics.families),
  }))
  const peak = Math.max(0, ...buckets.map((bucket) => bucket.count ?? 0))
  const maxCalls = Math.max(
    0,
    ...operations.map((operation) => operation.total),
  )
  const rows = slowOperationBuckets(operations, metrics.families).map((row) => {
    const operation = operations.find((one) => one.op === row.filter.method)
    return {
      ...row,
      failed: operation
        ? failedDurationBandCount(metrics.families, operation, row.filter)
        : null,
    }
  })
  return (
    <>
      <section
        aria-label="Metric totals"
        className="grid grid-cols-2 gap-5 border-line border-y pt-[15px] pb-[19px] lg:grid-cols-4"
      >
        <MetricTotal
          label="Operations"
          value={count(calls)}
          detail={`Across ${plural(operations.length, 'method')}`}
        />
        <MetricTotal
          label="Over 500 ms"
          value={slow === null || measured === 0 ? '—' : count(slow)}
          detail={
            measured === 0
              ? 'No duration samples'
              : slow === null
                ? 'Required histogram bound unavailable'
                : `${Math.round((slow / measured) * 100)}% of measured operations`
          }
        />
        <MetricTotal
          label="Failed operations"
          value={count(failed)}
          detail="Across all duration ranges"
          bad={failed > 0}
        />
        <MetricTotal
          label="Completed runs"
          value={count(runs.total)}
          detail={`${count(runs.byStatus.ok ?? 0)} succeeded · ${count(runs.inFlight)} in flight`}
        />
      </section>
      <div className="grid gap-7 pt-5 xl:grid-cols-2">
        <section
          aria-label="Operation duration"
          className="min-w-0 rounded-[7px] border border-line p-5"
        >
          <SectionTitle
            title="Operation duration"
            detail="Number of calls in each latency range"
          />
          {measured === 0 ? (
            <Notice>No operation duration samples have been recorded.</Notice>
          ) : (
            <>
              <div className="mt-3 flex flex-col gap-3">
                {buckets.map((bucket) => (
                  <div
                    key={bucket.label}
                    data-duration-range={bucket.label}
                    className={`flex min-h-[29px] items-center gap-3.5 rounded px-2 font-code text-[13px] ${bucket.minMs !== null && bucket.minMs >= 500 && (bucket.count ?? 0) > 0 ? 'bg-neo-tint text-neo' : 'text-ink-2'}`}
                  >
                    <span className="w-[142px] shrink-0">{bucket.label}</span>
                    <span
                      className="flex h-3 min-w-0 flex-1 border-line border-l"
                      aria-hidden="true"
                    >
                      {bucket.count !== null && bucket.count > 0 && (
                        <span
                          className={`h-3 rounded-sm ${bucket.minMs !== null && bucket.minMs >= 500 ? 'bg-neo' : 'bg-neo-soft'}`}
                          style={{ width: `${(bucket.count / peak) * 100}%` }}
                        />
                      )}
                    </span>
                    <span
                      className="w-16 shrink-0 text-right font-medium"
                      title={
                        bucket.count === null
                          ? 'Neo did not supply the required histogram bounds.'
                          : undefined
                      }
                    >
                      {bucket.count === null ? 'Unknown' : count(bucket.count)}
                    </span>
                  </div>
                ))}
              </div>
              <p className="mt-3 text-[13px] text-ink-2">
                {plural(measured, 'duration sample')}. Ranges exclude the lower
                bound and include the upper bound.
              </p>
              {buckets.some((bucket) => bucket.count === null) && (
                <p className="mt-1 text-[13px] text-ink-2">
                  Unknown ranges require a bucket bound Neo did not supply.
                </p>
              )}
            </>
          )}
        </section>
        <section
          aria-label="Outcomes by operation"
          className="min-w-0 rounded-[7px] border border-line p-5"
        >
          <SectionTitle
            title="Outcomes by operation"
            detail="Calls by outcome per method"
          />
          {operations.length === 0 ? (
            <Notice>No operation has settled since Neo started.</Notice>
          ) : (
            <>
              <div className="mt-3 flex flex-col gap-3">
                {operations.map((operation) => {
                  const outcomes = operationOutcomes(
                    metrics.families,
                    operation,
                  )
                  const nonFailed =
                    outcomes.kind === 'labeled'
                      ? outcomes.ok
                      : outcomes.nonFailed
                  const error =
                    outcomes.kind === 'labeled'
                      ? outcomes.error
                      : outcomes.failed
                  const cancelled =
                    outcomes.kind === 'labeled' ? outcomes.cancelled : 0
                  return (
                    <div
                      key={operation.op}
                      data-operation={operation.op}
                      className="flex min-h-[29px] flex-wrap items-center gap-x-3 gap-y-1 text-[13px]"
                    >
                      <span
                        className="w-[190px] shrink-0 truncate font-code"
                        title={operation.op}
                      >
                        {operation.op}
                      </span>
                      <span
                        className="flex h-3 min-w-12 flex-1"
                        aria-hidden="true"
                      >
                        <span
                          className="flex h-3 overflow-hidden rounded-sm"
                          style={{
                            width: `${maxCalls > 0 ? (operation.total / maxCalls) * 100 : 0}%`,
                          }}
                        >
                          <span
                            className="h-3 bg-[#7896ab]"
                            style={{
                              width: `${operation.total > 0 ? (nonFailed / operation.total) * 100 : 0}%`,
                            }}
                          />
                          <span
                            className="h-3 bg-bad"
                            style={{
                              width: `${operation.total > 0 ? (error / operation.total) * 100 : 0}%`,
                            }}
                          />
                          {cancelled > 0 && (
                            <span
                              className="h-3 bg-busy"
                              style={{
                                width: `${(cancelled / operation.total) * 100}%`,
                              }}
                            />
                          )}
                        </span>
                      </span>
                      <span className="w-[150px] shrink-0 text-ink-2">
                        {count(nonFailed)}{' '}
                        {outcomes.kind === 'labeled'
                          ? 'succeeded'
                          : 'non-failed'}
                        {error > 0 && (
                          <span className="text-bad">
                            {' '}
                            · {count(error)} failed
                          </span>
                        )}
                        {cancelled > 0 && (
                          <span className="text-busy-ink">
                            {' '}
                            · {count(cancelled)} cancelled
                          </span>
                        )}
                      </span>
                    </div>
                  )
                })}
              </div>
              <p className="mt-3 text-[13px] text-ink-2">
                {plural(calls, 'call')} total
              </p>
            </>
          )}
        </section>
      </div>
      <section aria-label="Slow operations" className="pt-5">
        <div className="mb-2 flex flex-wrap items-center gap-3">
          <h2 className="font-semibold text-[21px] leading-[29px]">
            Slow operations
          </h2>
          <span className="rounded bg-panel px-[9px] py-1 text-[13px] text-ink-2">
            Over 500 ms
          </span>
          <span className="ml-auto text-ink-2 text-sm">
            {plural(
              new Set(rows.map((row) => row.filter.method)).size,
              'method',
            )}
          </span>
        </div>
        {rows.length === 0 ? (
          <Notice>
            {measured === 0
              ? 'No duration samples to investigate yet.'
              : slow === null
                ? 'Neo has not supplied a populated duration bucket wholly above 500 ms.'
                : 'No measured operation took over 500 ms.'}
          </Notice>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[820px] text-left text-sm">
              <thead className="bg-panel text-ink-2 text-xs uppercase tracking-wide">
                <tr>
                  <th className="px-3.5 py-2.5 font-semibold">Method</th>
                  <th className="px-3.5 py-2.5 font-semibold">
                    Duration bucket
                  </th>
                  <th className="px-3.5 py-2.5 font-semibold">Calls</th>
                  <th className="px-3.5 py-2.5 font-semibold">
                    Failed in bucket
                  </th>
                  <th className="px-3.5 py-2.5 font-semibold">Investigate</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr
                    key={`${row.filter.method}:${row.filter.minMs}:${row.filter.maxMs}`}
                    className="border-line border-b"
                  >
                    <td className="px-3.5 py-4 font-code font-medium">
                      {row.filter.method}
                    </td>
                    <td className="px-3.5 py-4 font-code text-[13px]">
                      {row.label}
                    </td>
                    <td className="px-3.5 py-4">{count(row.count)}</td>
                    <td
                      className={`px-3.5 py-4 ${row.failed !== null && row.failed > 0 ? 'text-bad' : 'text-ink-2'}`}
                      title={
                        row.failed === null
                          ? 'Outcome labels or matching histogram bounds are unavailable.'
                          : undefined
                      }
                    >
                      {row.failed === null ? 'Unknown' : count(row.failed)}
                    </td>
                    <td className="px-3.5 py-4">
                      <button
                        type="button"
                        className="whitespace-nowrap font-semibold text-neo hover:underline"
                        aria-label={`Find traces for ${row.filter.method}, ${row.label}`}
                        onClick={() => onFindTraces(row.filter)}
                      >
                        Find matching traces →
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <p className="mt-2 text-[13px] text-ink-2">
          Bucket failures use outcome-labeled duration samples. Unknown means
          the labels or matching bounds are unavailable. Matching traces cover
          runs retained by this viewer.
        </p>
        {slow === null && measured > 0 && (
          <p className="mt-1 text-[13px] text-ink-2">
            Some histogram bounds are unavailable. Only known slow intervals are
            listed.
          </p>
        )}
      </section>
    </>
  )
}

function MetricTotal({
  label,
  value,
  detail,
  bad = false,
}: {
  label: string
  value: string
  detail: string
  bad?: boolean
}) {
  return (
    <div>
      <h2 className="font-semibold text-ink-2 text-sm leading-[19px]">
        {label}
      </h2>
      <p
        className={`font-semibold text-[32px] leading-[44px] ${bad ? 'text-bad' : 'text-ink'}`}
      >
        {value}
      </p>
      <p className="text-[13px] text-ink-2 leading-[18px]">{detail}</p>
    </div>
  )
}

function SectionTitle({ title, detail }: { title: string; detail: string }) {
  return (
    <>
      <h2 className="font-semibold text-[21px] leading-[29px]">{title}</h2>
      <p className="mt-3 text-ink-2 text-sm leading-[19px]">{detail}</p>
    </>
  )
}

function Notice({ children }: { children: ReactNode }) {
  return (
    <p className="border-line border-b py-6 text-ink-2 text-sm leading-5">
      {children}
    </p>
  )
}
