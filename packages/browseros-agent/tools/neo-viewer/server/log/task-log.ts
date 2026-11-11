/**
 * The viewer's durable log: one append-only JSONL file per task, `<dataDir>/tasks/<taskId>.jsonl`.
 * The first line describes the task; each later line records one thing as it happened (a run
 * started, a run ended, the tools/list size), so a restart, or a crash mid-run, loses nothing
 * that was written. Appends to one file are queued so lines land in the order they were made.
 */

import { appendFile, mkdir, readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import type {
  ClientInfo,
  RunDetail,
  RunStatus,
  TaskIdentification,
  TextSize,
} from '../../shared/api'

export interface StoredTask {
  id: string
  name: string
  neoTaskId: string
  client: ClientInfo
  identifiedBy: TaskIdentification
  /** The task rule's key (`task-rule.ts`), so a restart keeps grouping calls the same way. */
  key: string
  agentKey: string
  createdAt: string
}

export interface StoredRunStart {
  id: string
  seq: number
  tool: string
  code: string
  timeoutMs: number | null
  taskIdStamped: boolean
  sentAt: string
  proxyMs: number
  gapMs: number | null
  request: RunDetail['request']
}

export interface StoredRunEnd {
  status: Exclude<RunStatus, 'running' | 'lost'>
  answeredAt: string
  reply: RunDetail['reply']
}

export type LogLine =
  | { type: 'task'; v: 1; task: StoredTask }
  | { type: 'tools-list'; at: string; size: TextSize }
  | { type: 'run-start'; run: StoredRunStart }
  | { type: 'run-end'; id: string; end: StoredRunEnd }

export class TaskLog {
  private readonly queues = new Map<string, Promise<void>>()

  private constructor(private readonly dir: string) {}

  /** Opens (creating) the log under `dataDir` and reads back every task's lines. */
  static async open(
    dataDir: string,
  ): Promise<{ log: TaskLog; tasks: LogLine[][] }> {
    const dir = join(dataDir, 'tasks')
    await mkdir(dir, { recursive: true })
    const tasks: LogLine[][] = []
    for (const name of (await readdir(dir)).sort()) {
      if (!name.endsWith('.jsonl')) continue
      const lines = parseLines(await readFile(join(dir, name), 'utf8'))
      if (lines[0]?.type === 'task') tasks.push(lines)
    }
    return { log: new TaskLog(dir), tasks }
  }

  append(taskId: string, line: LogLine): Promise<void> {
    const file = join(this.dir, `${taskId}.jsonl`)
    const previous = this.queues.get(taskId) ?? Promise.resolve()
    const next = previous
      .then(() => appendFile(file, `${JSON.stringify(line)}\n`))
      .catch((error) =>
        console.error(`neo-viewer: could not write ${file}: ${String(error)}`),
      )
    this.queues.set(taskId, next)
    return next
  }

  /** Waits for every queued line to be written. */
  async flush(): Promise<void> {
    await Promise.all(this.queues.values())
  }
}

/** Every complete line; a line cut by a crash mid-write is skipped. */
function parseLines(text: string): LogLine[] {
  const lines: LogLine[] = []
  for (const line of text.split('\n')) {
    if (line.trim() === '') continue
    try {
      lines.push(JSON.parse(line) as LogLine)
    } catch {
      // the torn last line of a crash
    }
  }
  return lines
}
