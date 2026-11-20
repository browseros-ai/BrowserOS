/**
 * An export's README.md: the one small file an agent starts at. It says what the export is, gives
 * the totals the page shows and one table row per run linking to that run's files, then explains
 * every file and field, so the agent opens only what a question needs. It stays under about 300
 * lines however many runs there are: past `TABLE_ROWS` runs the table points to session.json.
 */

import {
  cell,
  outcomeWord,
  runNumber,
  statementPreview,
} from '../../shared/run-markdown'
import { runTotals } from '../../web/screen/measures'
import { clock, count, plural, seconds } from '../../web/screen/units'
import type { ExportSource, MetricsFiles, RunFiles } from './write-export'

/** Characters of the reply a run's Markdown shows inline; the rest is in its reply file. */
export const REPLY_INLINE_CHARS = 2_000

/** The most runs the README lists one by one. */
export const TABLE_ROWS = 200

export function exportReadme(input: {
  source: ExportSource
  runFiles: Map<string, RunFiles>
  now: number
  dir: string
  width: number
  metricsFiles: MetricsFiles
}): string {
  const { source, runFiles, now, dir, width, metricsFiles } = input
  const { info, runs } = source
  const totals = runTotals(runs)
  const failedRuns = runs.filter((run) => outcomeWord(run) === 'failed').length
  const neo = info.container
    ? `\`${info.container.image}\`, container \`${info.container.name}\`, which the viewer started`
    : `managed outside the viewer; MCP was forwarded to ${info.upstreamUrl}`
  const out: string[] = [
    `# Neo viewer export: ${plural(runs.length, 'run')}`,
    '',
    `Every \`run\` call agents sent through the Neo viewer's MCP endpoint since the last Clear, in the order each reached the viewer, with what the model read back and Neo's own record of each run: its operations (the SDK methods the code called), their timing, outcome and errors.`,
    '',
    `- Exported: ${localTime(now)}, to \`${dir}\``,
    `- Runs since: ${localTime(Date.parse(source.log.clearedAt))} (the last Clear, or the viewer's start)`,
    `- Viewer: instance \`${source.instance}\`, commit ${
      source.viewerCommit ? `\`${source.viewerCommit}\`` : 'unknown'
    }`,
    `- Neo SDK image: ${neo}`,
    '- Screenshots are not exported.',
    '',
    '## Totals',
    '',
    '| | |',
    '| --- | --: |',
    `| Runs | ${count(runs.length)}${failedRuns > 0 ? ` (${failedRuns} failed)` : ''} |`,
    `| Time in Neo | ${seconds(totals.neoMs)} |`,
    `| Round trips | ${seconds(totals.roundTripMs)} |`,
    `| Time between calls | ${seconds(totals.gapMs)} |`,
    `| Method calls | ${totals.calls === null ? '–' : count(totals.calls)} |`,
    `| Code tokens | ≈${count(totals.codeTokens)} |`,
    `| Reply tokens | ≈${count(totals.replyTokens)} |`,
  ]
  if (source.log.toolsList)
    out.push(
      `| tools/list the client read | ≈${count(source.log.toolsList.tokens)} tok |`,
    )
  out.push(
    '',
    'Tokens are estimates: characters ÷ 4. "Time in Neo" is Neo\'s record of each run (end − start), or the round trip less the viewer\'s own hold when Neo has no record. "Time between calls" is the time from a reply to the agent\'s next call: its thinking, other tools, waits.',
    '',
    '## Runs',
    '',
    'Times are local. "Calls" are the method calls Neo recorded (– when Neo has no record of the run). A Neo time marked * is the viewer\'s measure: Neo had no record of that run.',
    '',
    '| # | Started | Client | Tasks | Result | Round trip | Neo | Calls | Code tok | Reply tok | First statement |',
    '| --: | --- | --- | --- | --- | --: | --: | --: | --: | --: | --- |',
  )
  for (const run of runs.slice(0, TABLE_ROWS)) {
    const files = runFiles.get(run.id)
    const number = runNumber(run, width)
    out.push(
      `| [${number}](${files?.markdown ?? ''}) | ${clock(run.sentAt)} | ${cell(
        run.client?.name ?? '–',
      )} | ${tasksCell(run)} | ${outcomeWord(
        run,
      )} | ${run.roundTripMs === null ? '–' : seconds(run.roundTripMs)} | ${neoCell(
        run,
      )} | ${run.operationCount ?? '–'} | ≈${count(run.codeSize.tokens)} | ${
        run.replySize ? `≈${count(run.replySize.tokens)}` : '–'
      } | ${codeSpan(statementPreview(run.code))} |`,
    )
  }
  if (runs.length > TABLE_ROWS)
    out.push(
      '',
      `Runs ${TABLE_ROWS + 1} to ${runs.length} are not listed here, to keep this file short: they are in \`session.json\` (\`runs\`), and their files in \`runs/\`.`,
    )
  out.push('', ...metricsSection(source, metricsFiles))
  out.push('', ...HOW_TO_READ, '', ...QUESTIONS)
  return `${out.join('\n')}\n`
}

/** Neo's own numbers (its metrics), or why there are none. */
function metricsSection(source: ExportSource, files: MetricsFiles): string[] {
  const metrics = source.metrics
  if (!metrics)
    return [
      '## Neo metrics',
      '',
      'None: Neo sent no metrics (an image older than its telemetry, or no host API and no OTLP push), so there is no `metrics/` folder.',
    ]
  const view = metrics.metrics
  const since = metrics.health?.sinceClear
  const errors = view.errors
    .slice(0, 5)
    .map((entry) => `\`${entry.owner}/${entry.code}\` ×${count(entry.count)}`)
    .join(', ')
  return [
    '## Neo metrics',
    '',
    `From Neo's ${view.source === 'scrape' ? 'host API (`GET /v1/metrics`)' : 'OTLP push'} at ${localTime(view.observedAt)}; counts are since Neo started unless marked.`,
    '',
    '| | |',
    '| --- | --: |',
    `| Runs | ${count(view.runs.total)}${since ? ` (${count(since.runs)} since the last Clear, ${count(since.runErrors)} failed)` : ''} |`,
    `| Operations | ${count(view.operations.reduce((total, op) => total + op.total, 0))}${since ? ` (${count(since.ops)} since the last Clear, ${count(since.opErrors)} failed)` : ''} |`,
    `| Errors | ${errors || 'none'} |`,
    `| Tabs open | ${count(Object.values(view.tabs.openByOrigin).reduce((total, n) => total + n, 0))} |`,
    `| Renderer crashes | ${count(view.crashes)} |`,
    '',
    `Files: ${[files.summary, files.prometheus]
      .filter((file): file is string => file !== null)
      .map((file) => `\`${file}\``)
      .join(', ')}.`,
  ]
}

/** The run's tasks: the script's, then the host's marked "(host)"; "–" for none. */
function tasksCell(run: ExportSource['runs'][number]): string {
  const ids = [
    ...run.tasks.map((task) => task.id),
    ...(run.agentTaskId ? [`${run.agentTaskId} (host)`] : []),
  ]
  return ids.length === 0 ? '–' : cell(ids.join(', '))
}

function neoCell(run: ExportSource['runs'][number]): string {
  if (run.neoDurationMs !== null) return seconds(run.neoDurationMs)
  return run.roundTripMs === null
    ? '–'
    : `${seconds(Math.max(0, run.roundTripMs - run.proxyMs))}*`
}

/** Code in a table cell: a code span, unless it holds a backtick. */
function codeSpan(text: string): string {
  if (text === '') return '–'
  return text.includes('`') ? cell(text) : `\`${cell(text)}\``
}

/** `2026-11-11 22:45:01 (UTC-08:00)`. */
function localTime(at: number): string {
  const date = new Date(at)
  const pad = (n: number) => String(Math.abs(n)).padStart(2, '0')
  const offset = -date.getTimezoneOffset()
  const sign = offset < 0 ? '-' : '+'
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}:${pad(date.getSeconds())} (UTC${sign}${pad(
    Math.trunc(offset / 60),
  )}:${pad(offset % 60)})`
}

const HOW_TO_READ = [
  '## How to read this folder',
  '',
  "Start here; open a run's files only for the runs a question is about. `NNN` is the run number above, zero-padded.",
  '',
  '- **`README.md`**: this file.',
  '- **`session.json`**: the same facts as data.',
  '  - `export`: when, the folder, the viewer instance and commit, the Neo image and container (`neo.image` is null when the viewer forwarded to a Neo it did not start).',
  '  - `log.clearedAt`: when this list began. `totals`: the numbers above, in ms and token estimates.',
  "  - `toolsList`: the `tools/list` reply the agent read (the `run` tool's description and input schema, which sits in the model's context), with its size; null if no client asked during this log.",
  "  - `runs[]`: one summary per run, in arrival order: `number`, `seq`, `outcome` (`ok`, `failed`, `cancelled`, `lost`, `running`), `status` (the viewer's: `ok`, `error`, `rpc-error`, `http-error`, …), `client` (the agent's MCP client name), `tasks` (the tasks the run's script created or picked with `neo.createTask`/`neo.task`: `id`, and `name` when the reply carries one), `agentTaskId` (the host's task, from the request's `_meta` taskId, if any), `sentAt`/`answeredAt` (ISO), `roundTripMs`, `neoMs` (time in Neo, as above), `neoDurationMs` (Neo's own record only), `gapMs` (since the previous reply; null when runs overlapped), `proxyMs` (the viewer's hold), `operationCount`, `codeSize`/`replySize` (`chars`, `tokens`), `neoRunId`, `neoStatus`, `firstStatement`, and `files` (this run's paths). No code or replies inline.",
  `- **\`runs/NNN.md\`**: one run, top to bottom: a one-line header, the facts, the code laid out one statement per line as the viewer shows it (whitespace only; \`code\` in \`runs/NNN.json\` is as sent), each line's methods (\`L3 locator.click: +415 ms, 80 ms, ok\` is line 3, started 415 ms into the run, took 80 ms), a table of every operation (\`seq\`, \`name\`, \`effect\` read or write, \`line\`, \`start\` offset, \`duration\`, \`outcome\`, \`error\`), the operations not on a code line (Neo's own work, or calls inside another operation), the console lines, the error if it failed, and what the model read: the first ${count(REPLY_INLINE_CHARS)} characters, with the full size and a link to the reply file. A returned string (a snapshot, say) is shown decoded, as lines; its size and the reply file are what the model received, a JSON string.`,
  "- **`runs/NNN.json`**: that run complete. `code` as sent; `request.raw`, the HTTP body the agent sent; `reply.raw`, the body Neo answered with (SSE framing included); `reply.text`, what the model read; `reply.structuredContent`, `reply.isError`, `reply.meta`, `reply.rpcError`, `reply.notifications`; `parsed` (`value`, the returned value when `valueIsJson`; `valueText`; `console`; `notes`, Neo's `[unknown outcome]`/`[not awaited]` lines); `neo`, Neo's record (null when there is none): `status`, `startedAt`/`endedAt` (epoch ms), `durationMs`, `complete`, `tabs`, `error`, and `operations[]` with each one's `seq`, `parentId`, `name`, `kind`, `effect`, `site` (`line`, `column`), `startOffsetMs`, `durationMs`, `tabId`, `args`, `result`, `argsTruncated`/`resultTruncated` (Neo bounds both), `outcome`, `delivery`, `error` (`owner`, `code`, `message`, `retryable`) and `notes` (its call log).",
  '- **`runs/NNN.reply.txt`** (`.reply.json` when the whole text is JSON): exactly the text the model read, alone, to grep. Absent when the model read nothing.',
  "- **`log.jsonl`**: the viewer's raw log since the last Clear, one JSON object per line: `log` (its start), `run-start`, `run-end`, `neo-run` (a later one replaces an earlier one for the same run), `tools-list`.",
  "- **`metrics/metrics.prom`**: Neo's last answer to `GET /v1/metrics` (Prometheus text, byte for byte), the same sample `summary.json` reads; absent when those numbers came by push (no host API, or its scrape had gone quiet). **`metrics/summary.json`**: the same metrics read out: `health` (`sinceClear` counts since the last Clear; `runsInFlight`, `tabsOpen` live), `metrics` (runs by status with duration p50/p95, operations by name with errors and p50/p95, errors by owner and code, tabs by origin and close reason, crashes; cumulative since Neo started), `history` (one point a second, the last ten minutes, cumulative) and `families` (every metric as received). Absent when Neo sent no metrics.",
  "- A run's `neo` record in `runs/NNN.json` says where it came from: `source: \"spans\"` when the viewer had no host API and built it from Neo's OpenTelemetry spans (no args, results or notes); absent when it is Neo's own `/v1/runs` record.",
]

const QUESTIONS = [
  '## Questions this can answer',
  '',
  "- **Slowest runs**: sort `session.json` by `neoMs`: `jq '.runs | sort_by(-.neoMs) | .[:5] | map({number, neoMs, firstStatement})' session.json`; then the run's \"Methods by line\" says which line took the time.",
  "- **Biggest replies** (what costs the model's context): `jq '.runs | sort_by(-(.replySize.tokens // 0)) | .[:5] | map({number, replySize, files})' session.json`.",
  '- **Failed operations**: `jq -r \'.neo.operations[]? | select(.outcome == "error") | "\\(.seq) \\(.name) \\(.error.code): \\(.error.message)"\' runs/*[0-9].json`; or the `failed` rows above and each run\'s "Error".',
  "- **Repeated code**: `jq -r '.runs[].firstStatement' session.json | sort | uniq -c | sort -rn`; compare whole scripts with `jq -r .code runs/NNN.json`.",
  '- **Time the agent spent between calls**: `gapMs` per run, and the total above.',
  '- **Where a value came from**: grep the reply files, `grep -l "text" runs/*.reply.*`.',
]
