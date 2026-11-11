/**
 * Every task and run the viewer knows, in memory, backed by the JSONL log (`task-log.ts`).
 *
 * The proxy reports calls here as they start and end; the HTTP API reads views of them
 * (`shared/api.ts` shapes) and `/api/events` relays each change. Derived numbers (round trip,
 * token estimates, totals, the Neo run id) are computed from the stored records when read, so
 * the log keeps only what was observed.
 */

import type {
  ClientInfo,
  RunDetail,
  RunStatus,
  RunSummary,
  TaskSummary,
  TaskTotals,
  TextSize,
  ViewerEvent,
} from '../../shared/api'
import { textSize } from '../../shared/api'
import { RUN_META_KEY } from '../proxy/request-body'
import {
  type Assignment,
  DEFAULT_IDLE_MS,
  type TaskRule,
} from '../tasks/task-rule'
import type {
  LogLine,
  StoredRunEnd,
  StoredRunStart,
  StoredTask,
} from './task-log'
import { TaskLog } from './task-log'

const PREVIEW_CHARS = 240

interface TaskState {
  record: StoredTask
  toolsList: TextSize | null
  runs: RunState[]
}

interface RunState {
  taskId: string
  start: StoredRunStart
  end: StoredRunEnd | null
  /** Started in this process and not ended yet; false for a run read back without an end. */
  live: boolean
}

/** A run as the proxy reports it starting (the store adds its `id`). */
export type RunStart = Omit<StoredRunStart, 'id'>

type ViewerChange =
  | { type: 'task'; task: TaskSummary }
  | { type: 'run'; run: RunSummary }

export class TaskStore {
  private readonly tasks = new Map<string, TaskState>()
  private readonly runs = new Map<string, RunState>()
  private readonly toolsListByAgent = new Map<string, TextSize>()
  private readonly listeners = new Set<(event: ViewerEvent) => void>()
  private arrivals = 0
  private changes = 0

  private constructor(private readonly log: TaskLog) {}

  /** Reads the log back and tells the task rule about each task, so calls keep grouping after a restart. */
  static async open(dataDir: string, rule: TaskRule): Promise<TaskStore> {
    const { log, tasks } = await TaskLog.open(dataDir)
    const store = new TaskStore(log)
    for (const lines of tasks) store.restore(lines, rule)
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

  /** Starts the task an assignment created, named `<client>, HH:MM`. */
  createTask(assignment: Assignment, client: ClientInfo, at: number): void {
    if (this.tasks.has(assignment.taskId)) return
    const record: StoredTask = {
      id: assignment.taskId,
      name: `${client.name}, ${clock(at)}`,
      neoTaskId:
        assignment.identifiedBy.kind === 'agent-task-id'
          ? assignment.identifiedBy.taskId
          : assignment.taskId,
      client,
      identifiedBy: assignment.identifiedBy,
      key: assignment.key,
      agentKey: assignment.agentKey,
      createdAt: new Date(at).toISOString(),
    }
    const toolsList = this.toolsListByAgent.get(assignment.agentKey) ?? null
    this.tasks.set(record.id, { record, toolsList, runs: [] })
    this.write(record.id, { type: 'task', v: 1, task: record })
    if (toolsList)
      this.write(record.id, {
        type: 'tools-list',
        at: record.createdAt,
        size: toolsList,
      })
    this.emitTask(record.id)
  }

  /**
   * A `tools/list` reply went to the agent behind `agentKey` (its process, or its client name).
   * Each task records the first one it can be tied to: a task this agent starts later, or one it
   * is working on now, whichever rule named it. The reply can be logged after the agent's first
   * run made its task, since logging waits for the process lookup.
   */
  toolsListSeen(agentKey: string, size: TextSize, at: number): void {
    this.toolsListByAgent.set(agentKey, size)
    for (const task of this.tasks.values()) {
      if (task.record.agentKey !== agentKey || task.toolsList) continue
      if (Math.abs(Date.parse(lastCallAt(task)) - at) > DEFAULT_IDLE_MS)
        continue
      task.toolsList = size
      this.write(task.record.id, {
        type: 'tools-list',
        at: new Date(at).toISOString(),
        size,
      })
      this.emitTask(task.record.id)
    }
  }

  /**
   * A ticket taken when a call reaches the viewer. A run is recorded once its task is known,
   * which can be after a later call's (the process lookup), so runs are ordered by when they
   * arrived, not by when they were recorded.
   */
  nextArrival(): number {
    return ++this.arrivals
  }

  startRun(taskId: string, start: RunStart): string {
    const task = this.tasks.get(taskId)
    if (!task) throw new Error(`no task ${taskId}`)
    const id = `${taskId}-r${task.runs.length + 1}`
    const run: RunState = {
      taskId,
      start: { ...start, id },
      end: null,
      live: true,
    }
    const at = task.runs.findIndex((other) => comesBefore(run, other))
    const index = at === -1 ? task.runs.length : at
    task.runs.splice(index, 0, run)
    this.runs.set(id, run)
    this.write(taskId, { type: 'run-start', run: run.start })
    // A run placed before others renumbers them.
    this.emitRunsFrom(task, index)
    this.emitTask(taskId)
    return id
  }

  endRun(runId: string, end: StoredRunEnd): void {
    const run = this.runs.get(runId)
    if (!run || run.end) return
    run.end = end
    run.live = false
    this.write(run.taskId, { type: 'run-end', id: runId, end })
    // Its reply can give later runs their gap (it may have been recorded after them).
    const task = this.tasks.get(run.taskId)
    if (task) this.emitRunsFrom(task, task.runs.indexOf(run))
    this.emitTask(run.taskId)
  }

  listTasks(): TaskSummary[] {
    return [...this.tasks.values()]
      .map((task) => summarizeTask(task))
      .sort((a, b) => b.createdAt.localeCompare(a.createdAt))
  }

  task(taskId: string): { task: TaskSummary; runs: RunSummary[] } | null {
    const task = this.tasks.get(taskId)
    if (!task) return null
    return { task: summarizeTask(task), runs: runViews(task) }
  }

  run(runId: string): RunDetail | null {
    const run = this.runs.get(runId)
    const task = run && this.tasks.get(run.taskId)
    const view =
      task && runViews(task).find((candidate) => candidate.id === runId)
    return run && view ? detailRun(run, view) : null
  }

  private restore(lines: LogLine[], rule: TaskRule): void {
    const header = lines[0]
    if (header?.type !== 'task') return
    const task: TaskState = { record: header.task, toolsList: null, runs: [] }
    const byId = new Map<string, RunState>()
    for (const line of lines.slice(1)) {
      if (line.type === 'tools-list') task.toolsList ??= line.size
      else if (line.type === 'run-start') {
        const run: RunState = {
          taskId: task.record.id,
          start: line.run,
          end: null,
          live: false,
        }
        task.runs.push(run)
        byId.set(line.run.id, run)
      } else if (line.type === 'run-end') {
        const run = byId.get(line.id)
        if (run) run.end = line.end
      }
    }
    task.runs.sort((a, b) =>
      comesBefore(a, b) ? -1 : comesBefore(b, a) ? 1 : 0,
    )
    this.tasks.set(task.record.id, task)
    for (const [id, run] of byId) this.runs.set(id, run)
    if (task.toolsList)
      this.toolsListByAgent.set(task.record.agentKey, task.toolsList)
    rule.restore(task.record.key, task.record.id, Date.parse(lastCallAt(task)))
  }

  private write(taskId: string, line: LogLine): void {
    void this.log.append(taskId, line)
  }

  private emitTask(taskId: string): void {
    const task = this.tasks.get(taskId)
    if (task) this.emit({ type: 'task', task: summarizeTask(task) })
  }

  private emitRunsFrom(task: TaskState, index: number): void {
    for (const view of runViews(task).slice(Math.max(0, index))) {
      this.emit({ type: 'run', run: view })
    }
  }

  private emit(event: ViewerChange): void {
    const numbered = { ...event, version: ++this.changes } as ViewerEvent
    for (const listener of this.listeners) listener(numbered)
  }
}

/** Arrival order: when the call reached the viewer, then its arrival ticket. */
function comesBefore(a: RunState, b: RunState): boolean {
  if (a.start.sentAt !== b.start.sentAt) return a.start.sentAt < b.start.sentAt
  return a.start.arrival < b.start.arrival
}

/**
 * The task's runs as listed: numbered in arrival order, each with the time since the latest
 * earlier reply. The gap is null for the first run, and for a run sent while an earlier one was
 * still going (they overlap; the agent was not waiting on a reply).
 */
function runViews(task: TaskState): RunSummary[] {
  let latestReply: number | null = null
  let earlierRunning = false
  return task.runs.map((run, index) => {
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
    taskId: run.taskId,
    seq,
    tool: start.tool,
    status: statusOf(run),
    code: start.code,
    timeoutMs: start.timeoutMs,
    taskIdStamped: start.taskIdStamped,
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
  }
}

function detailRun(run: RunState, view: RunSummary): RunDetail {
  return {
    ...view,
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

function summarizeTask(task: TaskState): TaskSummary {
  const { record } = task
  return {
    id: record.id,
    name: record.name,
    neoTaskId: record.neoTaskId,
    client: record.client,
    identifiedBy: record.identifiedBy,
    createdAt: record.createdAt,
    lastCallAt: lastCallAt(task),
    toolsList: task.toolsList,
    totals: totalsOf(runViews(task)),
  }
}

function totalsOf(runs: RunSummary[]): TaskTotals {
  const totals: TaskTotals = {
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

function lastCallAt(task: TaskState): string {
  let latest = task.record.createdAt
  for (const run of task.runs) {
    const at = run.end?.answeredAt ?? run.start.sentAt
    if (at > latest) latest = at
  }
  return latest
}

function runMetaOf(meta: unknown): Record<string, unknown> | null {
  if (typeof meta !== 'object' || meta === null) return null
  const run = (meta as Record<string, unknown>)[RUN_META_KEY]
  return typeof run === 'object' && run !== null
    ? (run as Record<string, unknown>)
    : null
}

/** `HH:MM` in local time, as the task name shows it. */
function clock(at: number): string {
  const date = new Date(at)
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}
