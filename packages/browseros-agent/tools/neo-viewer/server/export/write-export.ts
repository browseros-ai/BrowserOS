/**
 * Export: every run since the last Clear, written as a folder an agent can read from one small
 * entry point down to one run's full record, and open only what it needs (owner, 2026-11-11).
 *
 * ```text
 * /tmp/neo-viewer/<instance>/export-<YYYYMMDD-HHMMSS>/
 *   README.md            the entry point: what this is, totals, one row per run, how to read the rest
 *   session.json         the same facts as data, each run's file paths; no code or replies inline
 *   runs/NNN.md          one run, top to bottom (shared/run-markdown.ts, as the run row's Copy)
 *   runs/NNN.json        that run complete: request and reply as they crossed, Neo's record
 *   runs/NNN.reply.txt   the text the model read, alone (`.reply.json` when it parses)
 *   log.jsonl            the raw log, for exactness
 * ```
 *
 * The folder is outside the instance's state folder, so an export survives the viewer stopping
 * and Clear. Nothing the viewer holds for Neo's host API (its token, an Authorization header) is
 * in a run's detail, so none of it can reach an export; a test greps every file for the token.
 */

import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type {
  ExportResponse,
  LogSummary,
  RunDetail,
  ViewerInfo,
} from '../../shared/api'
import {
  firstStatement,
  outcomeWord,
  runMarkdown,
  runNumber,
} from '../../shared/run-markdown'
import { neoMs, runTotals } from '../../web/screen/measures'
import { replyParts } from '../../web/screen/reply'
import { exportReadme, REPLY_INLINE_CHARS } from './export-readme'

/** Where exports go: under the system temp folder, one folder per instance. */
const EXPORT_ROOT = '/tmp/neo-viewer'

export interface ExportSource {
  instance: string
  info: ViewerInfo
  /** The viewer's checkout commit (`abc123def`, `+dirty` when edited), or null outside git. */
  viewerCommit: string | null
  log: LogSummary
  runs: RunDetail[]
  /** The latest `tools/list` result a client read, or null. */
  toolsList: unknown
  /** Copies the raw log to the given path. */
  copyLog: (destination: string) => Promise<void>
}

/** A run's files, relative to the export folder. */
export interface RunFiles {
  markdown: string
  json: string
  /** Null when the model read nothing (no reply text). */
  reply: string | null
}

export async function writeExport(
  source: ExportSource,
  options: { root?: string; now?: number } = {},
): Promise<ExportResponse> {
  const now = options.now ?? Date.now()
  const dir = await newFolder(
    join(options.root ?? EXPORT_ROOT, source.instance),
    `export-${stamp(now)}`,
  )
  await mkdir(join(dir, 'runs'), { mode: 0o700 })
  const width = Math.max(3, String(source.runs.length).length)
  const files: string[] = ['README.md', 'session.json']
  const runFiles = new Map<string, RunFiles>()
  for (const run of source.runs) {
    const written = await writeRun(dir, run, width)
    runFiles.set(run.id, written)
    files.push(
      written.markdown,
      written.json,
      ...(written.reply ? [written.reply] : []),
    )
  }
  await source.copyLog(join(dir, 'log.jsonl'))
  files.push('log.jsonl')
  const session = sessionJson(source, runFiles, now, dir, width)
  await writeFile(
    join(dir, 'session.json'),
    `${JSON.stringify(session, null, 2)}\n`,
  )
  await writeFile(
    join(dir, 'README.md'),
    exportReadme({ source, runFiles, now, dir, width }),
  )
  return { dir, files, runs: source.runs.length }
}

async function writeRun(
  dir: string,
  run: RunDetail,
  width: number,
): Promise<RunFiles> {
  const number = runNumber(run, width)
  const text = run.reply.text
  const replyName =
    text === null ? null : `${number}.reply.${parses(text) ? 'json' : 'txt'}`
  const files: RunFiles = {
    markdown: `runs/${number}.md`,
    json: `runs/${number}.json`,
    reply: replyName ? `runs/${replyName}` : null,
  }
  await writeFile(
    join(dir, files.markdown),
    runMarkdown(run, {
      replyChars: REPLY_INLINE_CHARS,
      allOperations: true,
      numberWidth: width,
      ...(replyName ? { replyFile: replyName } : {}),
    }),
  )
  await writeFile(
    join(dir, files.json),
    `${JSON.stringify(completeRun(run), null, 2)}\n`,
  )
  if (files.reply && text !== null)
    await writeFile(join(dir, files.reply), text)
  return files
}

/** `runs/NNN.json`: the run's detail, plus the reply read into its parts. */
function completeRun(run: RunDetail) {
  const text = run.reply.text
  const parts = text === null ? null : replyParts(text)
  const value = parts && !run.reply.isError ? parseJson(parts.value) : null
  return {
    ...run,
    parsed: parts && {
      /** The returned value when it is JSON; null for an error line, or text that is not JSON. */
      value: value ? value.value : null,
      valueIsJson: value !== null,
      valueText: parts.value,
      console: parts.console,
      notes: parts.notes,
    },
  }
}

function sessionJson(
  source: ExportSource,
  runFiles: Map<string, RunFiles>,
  now: number,
  dir: string,
  width: number,
) {
  const totals = runTotals(source.runs)
  const { info } = source
  return {
    export: {
      format: 1,
      createdAt: new Date(now).toISOString(),
      dir,
      instance: source.instance,
      viewer: {
        commit: source.viewerCommit,
        url: info.mcpUrl.replace(/\/mcp$/, ''),
        startedAt: info.startedAt,
      },
      neo: {
        image: info.container?.image ?? null,
        container: info.container?.name ?? null,
        upstreamUrl: info.upstreamUrl,
      },
      screenshots: 'not exported',
    },
    log: { clearedAt: source.log.clearedAt, rawLog: 'log.jsonl' },
    totals: {
      runs: totals.runs,
      failed: source.runs.filter((run) => outcomeWord(run) === 'failed').length,
      neoMs: totals.neoMs,
      roundTripMs: totals.roundTripMs,
      gapMs: totals.gapMs,
      wallClockMs: source.log.totals.wallClockMs,
      methodCalls: totals.calls,
      codeTokens: totals.codeTokens,
      replyTokens: totals.replyTokens,
      tokens: 'estimates: characters / 4',
    },
    toolsList: source.log.toolsList && {
      size: source.log.toolsList,
      reply: source.toolsList,
    },
    runs: source.runs.map((run) => {
      const { code, replyPreview, ...summary } = run
      return {
        number: runNumber(run, width),
        outcome: outcomeWord(run),
        firstStatement: firstStatement(code),
        neoMs: neoMs(run),
        ...pick(summary),
        files: runFiles.get(run.id),
      }
    }),
  }
}

/** The summary fields session.json keeps for a run (the detail's bodies stay in its files). */
function pick(run: Omit<RunDetail, 'code' | 'replyPreview'>) {
  return {
    seq: run.seq,
    id: run.id,
    status: run.status,
    client: run.client,
    agentTaskId: run.agentTaskId,
    tool: run.tool,
    sentAt: run.sentAt,
    answeredAt: run.answeredAt,
    roundTripMs: run.roundTripMs,
    proxyMs: run.proxyMs,
    gapMs: run.gapMs,
    neoDurationMs: run.neoDurationMs,
    operationCount: run.operationCount,
    codeSize: run.codeSize,
    replySize: run.replySize,
    timeoutMs: run.timeoutMs,
    neoRunId: run.neoRunId,
    neoStatus: run.neoStatus,
    neoEffects: run.neoEffects,
  }
}

/** A new folder, never an existing one: two exports in one second get `-2`, `-3`. */
async function newFolder(parent: string, name: string): Promise<string> {
  // Only this user reads an export: it holds pages' text and the agent's code.
  await mkdir(parent, { recursive: true, mode: 0o700 })
  for (let attempt = 1; ; attempt++) {
    const dir = join(parent, attempt === 1 ? name : `${name}-${attempt}`)
    try {
      await mkdir(dir, { mode: 0o700 })
      return dir
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST' || attempt > 50)
        throw error
    }
  }
}

/** `20261111-224501`, local time, as the folder name. */
function stamp(at: number): string {
  const date = new Date(at)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}-${pad(
    date.getHours(),
  )}${pad(date.getMinutes())}${pad(date.getSeconds())}`
}

function parses(text: string): boolean {
  return parseJson(text) !== null
}

function parseJson(text: string): { value: unknown } | null {
  try {
    return { value: JSON.parse(text) }
  } catch {
    return null
  }
}
