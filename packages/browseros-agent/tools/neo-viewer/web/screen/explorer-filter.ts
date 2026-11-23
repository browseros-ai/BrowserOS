/** A metric bucket searches operation spans with duration in (minMs, maxMs]. */
export interface ExplorerSpanFilter {
  method: string
  minMs: number
  /** Null is the open-ended final bucket. */
  maxMs: number | null
}
