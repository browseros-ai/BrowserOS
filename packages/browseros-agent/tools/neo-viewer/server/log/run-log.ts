/**
 * The viewer's durable log: one append-only JSONL file, `<dataDir>/log.jsonl`, every call through
 * the proxy in arrival order. The first line starts the log (its id and when); each later line
 * records one thing as it happened (a run started, ended, Neo's record of it, a tools/list size),
 * so a restart, or a crash mid-run, loses nothing that was written. Clear starts a new file and
 * keeps the previous one, and only that one, as `log.prev.jsonl` in case of a mis-click.
 *
 * Appends and clears go through one queue, so lines land in the file they were made for, in order.
 */

import {
  appendFile,
  copyFile,
  readFile,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises'
import { join } from 'node:path'
import type {
  ClientInfo,
  NeoRunRecord,
  RunDetail,
  RunStatus,
  TextSize,
} from '../../shared/api'

export interface StoredRunStart {
  /** `<log id>-<n>`: unique across clears and restarts. */
  id: string
  /** The store's arrival ticket: orders runs that reached the viewer in the same millisecond. */
  arrival: number
  tool: string
  code: string
  timeoutMs: number | null
  client: ClientInfo | null
  agentTaskId: string | null
  sentAt: string
  proxyMs: number
  request: RunDetail['request']
}

export interface StoredRunEnd {
  status: Exclude<RunStatus, 'running' | 'lost'>
  answeredAt: string
  reply: RunDetail['reply']
}

export type LogLine =
  | { type: 'log'; v: 2; id: string; clearedAt: string }
  /** `reply`: the `tools/list` result the client read (absent in logs written before exports). */
  | { type: 'tools-list'; at: string; size: TextSize; reply?: unknown }
  | { type: 'run-start'; run: StoredRunStart }
  | { type: 'run-end'; id: string; end: StoredRunEnd }
  /** Neo's record of the run, from its host API; a later line replaces an earlier one. */
  | { type: 'neo-run'; id: string; record: NeoRunRecord }

export const LOG_FILE = 'log.jsonl'
const PREVIOUS_LOG_FILE = 'log.prev.jsonl'

export class RunLog {
  private queue: Promise<void> = Promise.resolve()

  private constructor(
    private readonly file: string,
    private readonly previous: string,
  ) {}

  /** Opens the log in `dataDir` and reads its lines back (a torn last line is ended and skipped). */
  static async open(
    dataDir: string,
  ): Promise<{ log: RunLog; lines: LogLine[] }> {
    const log = new RunLog(
      join(dataDir, LOG_FILE),
      join(dataDir, PREVIOUS_LOG_FILE),
    )
    let text = ''
    try {
      text = await readFile(log.file, 'utf8')
    } catch {
      return { log, lines: [] }
    }
    // A crash mid-write leaves a torn last line. End it, so the next append starts a line of its
    // own instead of joining the fragment and being lost with it.
    if (text !== '' && !text.endsWith('\n')) await appendFile(log.file, '\n')
    return { log, lines: parseLines(text) }
  }

  append(line: LogLine): Promise<void> {
    return this.enqueue(() =>
      appendFile(this.file, `${JSON.stringify(line)}\n`),
    )
  }

  /** Starts the file with `header` (a first start, or one the command found empty). */
  begin(header: LogLine): Promise<void> {
    return this.enqueue(() =>
      writeFile(this.file, `${JSON.stringify(header)}\n`),
    )
  }

  /** Keeps the current file as the one previous log and starts a new one with `header`. */
  clear(header: LogLine): Promise<void> {
    return this.enqueue(async () => {
      await rm(this.previous, { force: true })
      await rename(this.file, this.previous).catch(() => {})
      await writeFile(this.file, `${JSON.stringify(header)}\n`)
    })
  }

  /**
   * Copies the file as it stands after every write queued so far (an export's `log.jsonl`).
   * Unlike a write, a failed copy rejects, so the export can say so; later writes still run.
   */
  copyTo(destination: string): Promise<void> {
    const copied = this.queue.then(() => copyFile(this.file, destination))
    this.queue = copied.catch(() => {})
    return copied
  }

  /** Waits for every queued write. */
  flush(): Promise<void> {
    return this.queue
  }

  private enqueue(write: () => Promise<unknown>): Promise<void> {
    this.queue = this.queue.then(write).then(
      () => {},
      (error) =>
        console.error(
          `neo-viewer: could not write ${this.file}: ${String(error)}`,
        ),
    )
    return this.queue
  }
}

/** Every complete line; the torn last line of a crash is skipped. */
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
