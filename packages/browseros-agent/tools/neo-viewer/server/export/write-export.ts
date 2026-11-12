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
 * The folder is outside the instance's state folder (an overlap is refused), so an export
 * survives the viewer stopping and Clear. Nothing the viewer holds for Neo's host API (its token, an Authorization header) is
 * in a run's detail, so none of it can reach an export; a test greps every file for the token.
 */

import { lstat, mkdir, realpath, writeFile } from 'node:fs/promises'
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
} from 'node:path'
import type {
  ExportResponse,
  LogSummary,
  RunDetail,
  ViewerInfo,
} from '../../shared/api'
import {
  modelReadText,
  outcomeWord,
  runMarkdown,
  runNumber,
  statementPreview,
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
  /** The raw log, taken with the runs (`RunStore.snapshot`). */
  rawLog: string
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
  const parent = join(options.root ?? EXPORT_ROOT, source.instance)
  await refuseStateFolder(parent, source.info.dataDir)
  const dir = await newFolder(parent, `export-${stamp(now)}`)
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
  await writeFile(join(dir, 'log.jsonl'), source.rawLog)
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
  const text = modelReadText(run)
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
      tips: parts.tips,
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
        firstStatement: statementPreview(code, 120),
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
    tasks: run.tasks,
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

/**
 * An export must outlive the viewer's state and its Clear, so it may not sit in the state folder,
 * nor the state folder in it (`--data-dir /tmp/neo-viewer` would put one inside the other).
 */
async function refuseStateFolder(
  parent: string,
  stateDir: string,
): Promise<void> {
  const [exports, state] = await Promise.all([
    canonical(parent),
    canonical(stateDir),
  ])
  const inside = (child: string, of: string) => {
    const path = relative(of, child)
    return path === '' || (!path.startsWith('..') && !isAbsolute(path))
  }
  if (inside(exports, state) || inside(state, exports))
    throw new Error(
      `the export folder ${parent} and the state folder ${stateDir} overlap; give the viewer a --data-dir outside ${parent}`,
    )
}

/** The path with symlinks resolved as far as it exists (`/tmp` is `/private/tmp` on macOS). */
async function canonical(path: string): Promise<string> {
  const absolute = resolve(path)
  try {
    return await realpath(absolute)
  } catch {
    const up = dirname(absolute)
    return up === absolute
      ? absolute
      : join(await canonical(up), basename(absolute))
  }
}

/** A new folder, never an existing one: two exports in one second get `-2`, `-3`. */
async function newFolder(parent: string, name: string): Promise<string> {
  // Only this user reads an export: it holds pages' text and the agent's code. Each folder is
  // checked before anything is made inside it, so a linked one never receives even a folder.
  await mkdir(dirname(dirname(parent)), { recursive: true })
  await privateFolder(dirname(parent))
  await privateFolder(parent)
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

/**
 * `/tmp` is shared: another user could have made `/tmp/neo-viewer` first and, owning it, swap the
 * export's folder for one of theirs while it is written. So each folder the export path runs
 * through below `/tmp` must be a real folder (not a link), this user's, and writable by no one else.
 */
async function privateFolder(dir: string): Promise<void> {
  try {
    await mkdir(dir, { mode: 0o700 })
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
  }
  await requirePrivate(dir)
}

async function requirePrivate(dir: string): Promise<void> {
  const info = await lstat(dir)
  const uid = process.getuid?.()
  if (
    !info.isDirectory() ||
    info.isSymbolicLink() ||
    (uid !== undefined && info.uid !== uid) ||
    (info.mode & 0o022) !== 0
  )
    throw new Error(
      `${dir} is not a private folder of this user (owner ${info.uid}, mode ${(info.mode & 0o777).toString(8)}); remove it, then export again`,
    )
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
