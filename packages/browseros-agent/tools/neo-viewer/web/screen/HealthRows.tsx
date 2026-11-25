/**
 * The Health view's table pieces (`HealthView`): a section with its headline figures on the left,
 * as the time strip has them; column labels in the run block's label type; rows on one column
 * grid; a count with its bar (its failed share red at the end), as the run table draws Neo time;
 * and a histogram at a glance, one column per bucket of Neo's own bounds.
 */

import type { ReactNode } from 'react'
import type { Quantiles } from '../../shared/api'
import { share } from './measures'
import { axisEnds, bucketCounts, bucketsTitle } from './metrics'
import { count } from './units'

/** The grid every section's rows share. */
export const NAME = 'w-[196px] shrink-0 truncate pr-3'
export const BARS = 'flex w-[170px] shrink-0 items-center gap-2'
export const FAILED = 'w-[64px] shrink-0 text-right'
export const QUANTILE = 'w-[72px] shrink-0 text-right'
export const SPREAD = 'ml-6 w-24 shrink-0'

/** A section: its title and headline figures on the left, as the time strip has them; its rows. */
export function Section({
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
export function Columns({
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

export function Row({
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
export function Empty({ children }: { children: ReactNode }) {
  return <p className="pt-px text-ink-2 text-xs leading-4">{children}</p>
}

/** A figure in a summary line, mono as the time strip's are. */
export function Figure({
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
export function Bars({
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
export function Failed({ value }: { value: number }) {
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
export function Spread({ quantiles }: { quantiles: Quantiles }) {
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
