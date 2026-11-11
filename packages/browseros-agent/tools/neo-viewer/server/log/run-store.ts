/**
 * Every run since the last Clear, in memory, backed by the JSONL log (`run-log.ts`).
 *
 * The proxy reports calls here as they start and end; the HTTP API reads views of them
 * (`shared/api.ts` shapes) and `/api/events` relays each change. One list for every agent: runs
 * are numbered by when they reached the viewer, whichever process or client sent them. Derived
 * numbers (sequence, gap, round trip, token estimates, totals) are computed when read, so the log
 * keeps only what was observed.
 */

import { randomBytes } from 'node:crypto'
import type {
  LogSummary,
  LogTotals,
  NeoRunRecord,
  RunDetail,
  RunStatus,
  RunSummary,
  TextSize,
  ViewerEvent,
} from '../../shared/api'
import { textSize } from '../../shared/api'
import { RUN_META_KEY } from '../proxy/request-body'
import {
  type LogLine,
  RunLog,
  type StoredRunEnd,
  type StoredRunStart,
} from './run-log'

const PREVIEW_CHARS = 240

interface RunState {
  start: StoredRunStart
  end: StoredRunEnd | null
  /** Started in this process and not ended yet; false for a run read back without an end. */
  live: boolean
  /** Neo's record of the run (`neo-runs.ts`), once read. */
  neo: NeoRunRecord | null
}

/** A run as the proxy reports it starting (the store adds its `id`). */
export type RunStart = Omit<StoredRunStart, 'id'>

type Change =
  | { type: 'run'; run: RunSummary }
  | { type: 'log'; log: LogSummary }
  | { type: 'clear'; log: LogSummary }

export class RunStore {
  private runs: RunState[] = []
  private byId = new Map<string, RunState>()
  private toolsList: TextSize | null = null
  private logId = newLogId()
  private clearedAt = new Date().toISOString()
  private readonly listeners = new Set<(event: ViewerEvent) => void>()
  private arrivals = 0
  private changes = 0
  private count = 0

  private constructor(private readonly log: RunLog) {}

  /** Reads the log back; a missing or empty log starts a new one. */
  static async open(dataDir: string): Promise<RunStore> {
    const { log, lines } = await RunLog.open(dataDir)
    const store = new RunStore(log)
    if (lines[0]?.type === 'log') store.restore(lines)
    else
      await log.begin({
        type: 'log',
        v: 2,
        id: store.logId,
        clearedAt: store.clearedAt,
      })
    return store
  }

  subscribe(listener: (event: ViewerEvent) => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  flush(): Promise<void> {
    return this.log.flush()
  }

  /**
   * How many changes the store has announced. Each event carries its number and each snapshot
   * the number it includes, so a reader that fetched a snapshot while events streamed in keeps
   * only the events newer than it (`web/use-viewer.ts`).
   */
  get version(): number {
    return this.changes
  }

  /**
   * A ticket taken when a call reaches the viewer. Runs are ordered by when they arrived, not by
   * when they were recorded.
   */
  nextArrival(): number {
    return ++this.arrivals
  }

  startRun(start: RunStart): string {
    const id = `${this.logId}-${++this.count}`
    const run: RunState = {
      start: { ...start, id },
      end: null,
      live: true,
      neo: null,
    }
    const at = this.runs.findIndex((other) => comesBefore(run, other))
    const index = at === -1 ? this.runs.length : at
    this.runs.splice(index, 0, run)
    this.byId.set(id, run)
    void this.log.append({ type: 'run-start', run: run.start })
    // A run placed before others renumbers them.
    this.emitRunsFrom(index)
    this.emitLog()
    return id
  }

  endRun(runId: string, end: StoredRunEnd): void {
    const run = this.byId.get(runId)
    if (!run || run.end) return
    run.end = end
    run.live = false
    void this.log.append({ type: 'run-end', id: runId, end })
    // Its reply can give later runs their gap (it may have been recorded after them).
    this.emitRunsFrom(this.runs.indexOf(run))
    this.emitLog()
  }

  /** Neo's record of a run, read from its host API; replaces an earlier, incomplete one. */
  attachNeoRun(runId: string, record: NeoRunRecord): void {
    const run = this.byId.get(runId)
    if (!run) return
    run.neo = record
    void this.log.append({ type: 'neo-run', id: runId, record })
    this.emitRunsFrom(this.runs.indexOf(run))
  }

  hasNeoRun(runId: string): boolean {
    return this.byId.get(runId)?.neo != null
  }

  /** A `tools/list` reply went to a client: the log shows the latest one's size. */
  toolsListSeen(size: TextSize, at: number): void {
    this.toolsList = size
    void this.log.append({
      type: 'tools-list',
      at: new Date(at).toISOString(),
      size,
    })
    this.emitLog()
  }

  /**
   * Clear: an empty log from now on. The file so far is kept as the one previous log. A run still
   * in flight finishes unseen: its end finds no run to update.
   */
  async clear(at = Date.now()): Promise<void> {
    this.runs = []
    this.byId = new Map()
    this.toolsList = null
    this.count = 0
    this.logId = newLogId()
    this.clearedAt = new Date(at).toISOString()
    await this.log.clear({
      type: 'log',
      v: 2,
      id: this.logId,
      clearedAt: this.clearedAt,
    })
    this.emit({ type: 'clear', log: this.summary() })
  }

  /** Every run, in arrival order. */
  list(): RunSummary[] {
    return runViews(this.runs)
  }

  summary(): LogSummary {
    return {
      clearedAt: this.clearedAt,
      totals: totalsOf(this.list()),
      toolsList: this.toolsList,
    }
  }

  run(runId: string): RunDetail | null {
    const run = this.byId.get(runId)
    const view = run && this.list().find((candidate) => candidate.id === runId)
    return run && view ? detailRun(run, view) : null
  }

  private restore(lines: LogLine[]): void {
    for (const line of lines) {
      if (line.type === 'log') {
        this.logId = line.id
        this.clearedAt = line.clearedAt
      } else if (line.type === 'tools-list') this.toolsList = line.size
      else if (line.type === 'run-start') {
        const run: RunState = {
          start: line.run,
          end: null,
          live: false,
          neo: null,
        }
        this.runs.push(run)
        this.byId.set(line.run.id, run)
      } else if (line.type === 'run-end') {
        const run = this.byId.get(line.id)
        if (run) run.end = line.end
      } else if (line.type === 'neo-run') {
        const run = this.byId.get(line.id)
        if (run) run.neo = line.record
      }
    }
    this.runs.sort((a, b) =>
      comesBefore(a, b) ? -1 : comesBefore(b, a) ? 1 : 0,
    )
    this.count = this.runs.length
  }

  private emitRunsFrom(index: number): void {
    for (const view of this.list().slice(Math.max(0, index)))
      this.emit({ type: 'run', run: view })
  }

  private emitLog(): void {
    this.emit({ type: 'log', log: this.summary() })
  }

  private emit(change: Change): void {
    const event = { ...change, version: ++this.changes } as ViewerEvent
    for (const listener of this.listeners) listener(event)
  }
}

function newLogId(): string {
  return randomBytes(4).toString('hex')
}

/** Arrival order: when the call reached the viewer, then its arrival ticket. */
function comesBefore(a: RunState, b: RunState): boolean {
  if (a.start.sentAt !== b.start.sentAt) return a.start.sentAt < b.start.sentAt
  return a.start.arrival < b.start.arrival
}

/**
 * The runs as listed: numbered in arrival order, each with the time since the latest earlier
 * reply. The gap is null for the first run, and for a run sent while an earlier one was still
 * going (they overlap; that agent, or another, was not waiting on a reply).
 */
function runViews(runs: RunState[]): RunSummary[] {
  let latestReply: number | null = null
  let earlierRunning = false
  return runs.map((run, index) => {
    const sent = Date.parse(run.start.sentAt)
    const overlaps =
      earlierRunning || (latestReply !== null && latestReply > sent)
    const gap = latestReply === null || overlaps ? null : sent - latestReply
    if (run.end) {
      const answered = Date.parse(run.end.answeredAt)
      latestReply =
        latestReply === null ? answered : Math.max(latestReply, answered)
    } else if (run.live) {
      earlierRunning = true
    }
    return summarizeRun(run, index + 1, gap)
  })
}

function statusOf(run: RunState): RunStatus {
  if (run.end) return run.end.status
  return run.live ? 'running' : 'lost'
}

function summarizeRun(
  run: RunState,
  seq: number,
  gapMs: number | null,
): RunSummary {
  const { start, end } = run
  const text = end?.reply.text ?? null
  const runMeta = runMetaOf(end?.reply.meta)
  return {
    id: start.id,
    seq,
    client: start.client,
    agentTaskId: start.agentTaskId,
    tool: start.tool,
    status: statusOf(run),
    code: start.code,
    timeoutMs: start.timeoutMs,
    sentAt: start.sentAt,
    answeredAt: end?.answeredAt ?? null,
    roundTripMs: end
      ? Date.parse(end.answeredAt) - Date.parse(start.sentAt)
      : null,
    proxyMs: start.proxyMs,
    gapMs,
    codeSize: textSize(start.code),
    replySize: text === null ? null : textSize(text),
    replyPreview: text === null ? null : text.slice(0, PREVIEW_CHARS),
    neoRunId: typeof runMeta?.id === 'string' ? runMeta.id : null,
    neoStatus: typeof runMeta?.status === 'string' ? runMeta.status : null,
    neoEffects: typeof runMeta?.effects === 'string' ? runMeta.effects : null,
    neoDurationMs: run.neo?.durationMs ?? null,
    operationCount: run.neo ? run.neo.operations.length : null,
  }
}

function detailRun(run: RunState, view: RunSummary): RunDetail {
  return {
    ...view,
    neo: run.neo,
    request: run.start.request,
    reply: run.end?.reply ?? {
      httpStatus: null,
      contentType: null,
      contentEncoding: null,
      raw: null,
      text: null,
      structuredContent: null,
      isError: null,
      meta: null,
      rpcError: null,
      notifications: [],
      keepAlives: 0,
    },
  }
}

function totalsOf(runs: RunSummary[]): LogTotals {
  const totals: LogTotals = {
    runs: runs.length,
    running: 0,
    failed: 0,
    roundTripMs: 0,
    gapMs: 0,
    wallClockMs: 0,
    code: { chars: 0, tokens: 0 },
    reply: { chars: 0, tokens: 0 },
  }
  let first: number | null = null
  let last: number | null = null
  for (const run of runs) {
    if (run.status === 'running') totals.running++
    else if (run.status !== 'ok') totals.failed++
    totals.roundTripMs += run.roundTripMs ?? 0
    totals.gapMs += run.gapMs ?? 0
    add(totals.code, run.codeSize)
    if (run.replySize) add(totals.reply, run.replySize)
    const sent = Date.parse(run.sentAt)
    first = first === null ? sent : Math.min(first, sent)
    if (run.answeredAt) last = Math.max(last ?? 0, Date.parse(run.answeredAt))
  }
  if (first !== null && last !== null)
    totals.wallClockMs = Math.max(0, last - first)
  return totals
}

function add(total: TextSize, size: TextSize): void {
  total.chars += size.chars
  total.tokens += size.tokens
}

function runMetaOf(meta: unknown): Record<string, unknown> | null {
  if (typeof meta !== 'object' || meta === null) return null
  const run = (meta as Record<string, unknown>)[RUN_META_KEY]
  return typeof run === 'object' && run !== null
    ? (run as Record<string, unknown>)
    : null
}
