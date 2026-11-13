/** Neo records (`RunDetail.neo`) for the screen's tests, shaped as `server/neo-runs.ts` gives them. */

import type { NeoOperation, NeoRunRecord } from '../../shared/api'

export function operation(
  seq: number,
  name: string,
  options: Partial<NeoOperation> & { line?: number | null } = {},
): NeoOperation {
  const { line, ...rest } = options
  return {
    id: `neo-run.${seq}`,
    seq,
    parentId: null,
    name,
    kind: 'builtin',
    effect: 'read',
    site: line ? { line, column: 7 } : null,
    startOffsetMs: 0,
    durationMs: 1,
    tabId: 1,
    args: {},
    argsTruncated: false,
    result: null,
    resultTruncated: false,
    outcome: 'ok',
    delivery: 'not_sent',
    error: null,
    notes: [],
    hasScreenshot: false,
    ...rest,
  }
}

export function record(
  operations: NeoOperation[],
  durationMs: number | null = 100,
): NeoRunRecord {
  return {
    status: durationMs === null ? 'running' : 'ok',
    effects: null,
    startedAt: 0,
    endedAt: durationMs,
    durationMs,
    complete: durationMs !== null,
    tabs: [1],
    error: null,
    operations,
    traceGaps: 0,
  }
}
