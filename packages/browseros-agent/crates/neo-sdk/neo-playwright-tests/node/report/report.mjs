#!/usr/bin/env node
/** Neo: Read the two run artifacts and bridge JSONL, then emit a reviewable compatibility report. */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { parseArgs } from 'node:util'
import { scopeForRun, scoreResults, scoreText } from '../scope.mjs'
import {
  classifyRuns,
  compareBridge,
  countVerdicts,
  errorEvidence,
  verdicts,
} from './classify.mjs'
import { renderCuration } from './curation.mjs'
import { refusals } from './refusals.mjs'
import { clawRefusals } from './refusals-claw.mjs'

function cell(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('|', '&#124;')
    .replaceAll('`', '&#96;')
    .replaceAll('\n', '<br>')
}

function table(headers, rows) {
  return [headers, headers.map(() => '---'), ...rows]
    .map((row) => `| ${row.map(cell).join(' | ')} |`)
    .join('\n')
}

export function commandLine(argv) {
  if (typeof argv === 'string') return argv
  return argv
    .map((arg) =>
      /^[a-zA-Z0-9_./:=@,+-]+$/.test(arg)
        ? arg
        : `'${arg.replaceAll("'", "'\\''")}'`,
    )
    .join(' ')
}

function locationText(location) {
  return location
    ? `${location.file}:${location.line ?? '?'}:${location.column ?? '?'}`
    : 'not recorded'
}

function groupGaps(rows) {
  const groups = new Map()
  for (const row of rows.filter((row) =>
    ['neo-lacks', 'neo-differs', 'needs-internals'].includes(row.verdict),
  )) {
    const gap = groups.get(row.gapKey) || {
      key: row.gapKey,
      count: 0,
      examples: [],
    }
    gap.count++
    if (gap.examples.length < 3) gap.examples.push(`${row.suite}: ${row.title}`)
    groups.set(row.gapKey, gap)
  }
  return [...groups.values()].sort(
    (a, b) => b.count - a.count || a.key.localeCompare(b.key),
  )
}

/** Neo: The runner's `run.json.argv` holds only the arguments after `run.mjs`; empty is the full corpus. */
const RUNNER =
  'packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/run.mjs'

export function runnerCommand(argv) {
  if (typeof argv === 'string' || argv[0] === 'node') return commandLine(argv)
  return commandLine(['node', RUNNER, ...argv])
}

function runDetails(label, run) {
  const meta = run.metadata
  return [
    `### ${label}`,
    '',
    '```sh',
    runnerCommand(meta.argv),
    '```',
    '',
    `Directory: ${cell(run.directory)}. Commit: ${cell(meta.commit)}. Workers: ${cell(meta.workers)}.`,
    `Started: ${cell(meta.startedAt)}. Ended: ${cell(meta.endedAt)}. SDK SHA-256: ${cell(meta.sdkSha256)}.`,
    `Specs verified: ${cell(meta.specsVerified)}. Context option omissions: ${cell(JSON.stringify(meta.contextOptionOmissions || []))}.`,
    `expectLoop: ${cell(meta.expectLoop ?? false)}.`,
    ...(meta.engine
      ? [
          `Engine: ${cell(meta.engine)}. BrowserOS: ${cell(meta.browserosVersion)}. SDK sources: ${cell(JSON.stringify(meta.sdkSources || {}))}. Add-on: ${cell(JSON.stringify(meta.addon || {}))}.`,
          `Browser launch: ${cell(JSON.stringify(meta.browserLaunch || {}))}.`,
        ]
      : []),
    ...(meta.metadataSource
      ? [`Metadata provenance: ${cell(meta.metadataSource)}`]
      : []),
    '',
  ]
}

export function renderReport({ neo, reference, bridge, corpus = [], scope }) {
  const engine = neo.metadata.engine || neo.metadata.mode || 'neo'
  const label =
    engine === 'claw' ? 'v1' : engine === 'reference' ? 'Vanilla' : 'Neo'
  const refusalList = engine === 'claw' ? clawRefusals : refusals
  const rows = classifyRuns(neo.results, reference.results, corpus, { engine })
  const scores = scoreResults(neo.results, scope || scopeForRun(neo))
  const counts = countVerdicts(rows)
  const comparison = compareBridge(rows, bridge, corpus)
  const suites = [...new Set(rows.map((row) => row.suite))].sort()
  const expectOnly = rows.filter((row) => row.wouldPassWithExpect).length
  const flaky = rows.filter((row) => row.playwrightStatus === 'flaky').length
  const missingReference = rows.filter(
    (row) => row.referenceOutcome === 'missing',
  ).length
  const skippedReference = rows.filter(
    (row) => row.referenceOutcome === 'skip' && row.outcome !== 'skip',
  ).length
  const diagnostics = ['neo', 'reference'].flatMap(
    (mode) =>
      (mode === 'neo' ? neo : reference).results.errors?.map((error) => ({
        mode,
        ...errorEvidence(error),
      })) || [],
  )
  const gaps = groupGaps(rows)
  const skips = new Map()
  for (const row of rows.filter((row) => row.verdict === 'skip')) {
    const reason = row.skipReasons.join('; ') || 'No reason recorded'
    skips.set(reason, (skips.get(reason) || 0) + 1)
  }
  const lines = [
    `# ${label} in Node: Playwright Test verdicts`,
    '',
    `${rows.length} selected cases in ${suites.length} ${suites.length === 1 ? 'suite' : 'suites'}. Playwright expected outcomes (including expected failures) and flaky outcomes count as pass; skips stay skips. Raw attempt statuses and all error evidence are retained in JSONL.`,
    '',
    'Failure precedence: harness-error → invalid-here → needs-internals → neo-lacks → neo-differs. invalid-here means the reference also has an unexpected test outcome; expected reference failures remain passes.',
    '',
    `Flaky cases: ${flaky}. Missing reference matches: ${missingReference}. Reference skips for executed ${label} cases: ${skippedReference}. Unscoped runner errors: ${diagnostics.length}. Missing/skipped reference results do not establish invalid-here.`,
    ...(diagnostics.length
      ? [
          '',
          '**Run incomplete:** runner-level errors are listed below; completed per-case verdicts are preserved.',
        ]
      : []),
    ...(neo.results.config?.failOnFlakyTests && flaky
      ? [
          '',
          'The run uses failOnFlakyTests: its overall exit can fail even though Playwright marks these individual tests flaky.',
        ]
      : []),
    '',
    '## Scores',
    '',
    `All tests: **${scoreText(scores.all)}**. In-scope tests: **${scoreText(scores.inScope)}**.`,
    '',
    'Scores count Playwright expected/flaky outcomes as passing, including expectLoop passes. Skipped tests ran nothing: they are counted beside each score, split by who skipped them, and left out of its rate. Incomplete cases stay in. Scope changes only the denominator and matching pass count, never execution or verdicts. Counts below cover this selection.',
    '',
    neo.metadata.scope
      ? `Scope manifest SHA-256: ${neo.metadata.scope.manifestSha256} (recorded with this run).`
      : 'Scope provenance: current repository policy applied to a legacy run without a recorded scope file.',
    '',
    table(
      ['Scope reason', 'Excluded tests', 'Passing excluded tests'],
      Object.entries(scores.byReason).map(([reason, count]) => [
        reason,
        count.total,
        count.passed,
      ]),
    ),
    '',
    `### Passing tests in scope file (${scores.passingExcluded.length})`,
    '',
    ...(scores.passingExcluded.length
      ? [
          table(
            ['Test', 'Reason'],
            scores.passingExcluded.map((row) => [
              `${row.file}: ${row.title}`,
              row.reason,
            ]),
          ),
        ]
      : ['None.']),
    '',
    '## How to run',
    '',
    'Commands recorded in run.json.argv. Use the same prepared source and BrowserOS executable as these runs.',
    '',
    ...runDetails(label, neo),
    ...runDetails('Reference', reference),
    ...renderCuration(neo, rows),
    '## Totals by verdict',
    '',
    table(
      ['Verdict', 'Tests'],
      verdicts.map((verdict) => [verdict, counts[verdict]]),
    ),
    '',
    'Passing tests annotated with expectLoop count only as pass via expectLoop.',
    '',
    '## Skips by reason',
    '',
    ...(skips.size
      ? [
          table(
            ['Reason', 'Tests'],
            [...skips].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])),
          ),
        ]
      : ['None recorded.']),
    '',
    '## Per suite',
    '',
    table(
      ['Suite', ...verdicts],
      suites.map((suite) => {
        const total = countVerdicts(rows.filter((row) => row.suite === suite))
        return [suite, ...verdicts.map((verdict) => total[verdict])]
      }),
    ),
    '',
    '## Bridge comparison',
    '',
    `${comparison.matched}/${rows.length} selected Node cases matched the bridge by suite file and full title path. Bridge corpus: ${comparison.bridgeTotal} cases. Unmatched Node cases: ${comparison.unmatched}. Cases outside this Node selection have no measured transition.`,
    '',
    table(
      ['Bridge verdict → Node verdict', ...verdicts],
      Object.entries(comparison.matrix)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([prior, total]) => [
          prior,
          ...verdicts.map((verdict) => total[verdict]),
        ]),
    ),
    '',
    `Of the bridge's ${comparison.noVerdictTotal.toLocaleString('en-US')} no-verdict cases (BridgeLimit, CleanupBlur, HarnessError), **${comparison.recovered} now have a plain Playwright pass/fail** in this selection. ${comparison.noVerdictSelected} were selected; ${comparison.noVerdictSelected - comparison.recovered} remain skip/harness-error; ${comparison.noVerdictTotal - comparison.noVerdictSelected} were not selected. Plain pass/fail includes classified unexpected test failures, including needs-internals and invalid-here.`,
    '',
    `## Top ${label} gaps`,
    '',
    'Ranked by tests hit, one primary gap per case; includes matcher/fixture dependencies. Reference and infrastructure failures are excluded.',
    '',
    ...(gaps.length
      ? [
          table(
            ['Gap key', 'Tests', 'Example cases'],
            gaps.map((gap) => [gap.key, gap.count, gap.examples.join('\n')]),
          ),
        ]
      : ['None observed.']),
    '',
    '## Would pass with `_expect`',
    '',
    `**${expectOnly} cases** have only observed needs-internals failures for _expect / Frame.expect. This is a potential unlock count, not proof that later assertions would pass. Cases with another error (including _options.baseURL) are excluded.`,
    '',
    '## Failure evidence',
    '',
    table(
      ['Case', 'Verdict', 'Gap key', 'First error line', 'Location'],
      rows
        .filter(
          (row) =>
            !['pass', 'pass via expectLoop', 'skip'].includes(row.verdict),
        )
        .map((row) => [
          `${row.suite}: ${row.title}`,
          row.verdict,
          row.gapKey,
          row.evidence[0]?.firstLine || row.reason,
          locationText(row.evidence[0]?.location || row.location),
        ]),
    ),
    '',
    '## Runner errors',
    '',
    ...(diagnostics.length
      ? [
          table(
            ['Run', 'First error line', 'Location'],
            diagnostics.map((error) => [
              error.mode,
              error.firstLine,
              locationText(error.location),
            ]),
          ),
        ]
      : ['None recorded.']),
    '',
    `## Recognized ${label} refusals`,
    '',
    'Audited SDK/runtime message families. A generic “unsupported” message, a normal Playwright validation error, or text inside an assertion diff is insufficient. Missing functions must name a recognized Neo receiver; unknown receivers remain neo-differs.',
    '',
    table(
      ['Message family', 'Source under crates/neo-sdk'],
      refusalList.map((refusal) => [
        refusal.wording,
        refusal.sources.join('\n'),
      ]),
    ),
    '',
  ]
  return { markdown: lines.join('\n'), rows, counts, comparison, scores }
}

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch (error) {
    throw new Error(`Cannot read ${file}: ${error.message}`)
  }
}

function readRun(directory, mode) {
  const metadata = readJson(path.join(directory, 'run.json'))
  const argvValid =
    typeof metadata.argv === 'string'
      ? metadata.argv.length > 0
      : Array.isArray(metadata.argv) &&
        metadata.argv.every((arg) => typeof arg === 'string')
  const engine = metadata.engine || metadata.mode
  if (
    !(mode === 'neo'
      ? ['neo', 'claw', 'reference'].includes(engine)
      : engine === mode) ||
    !argvValid
  )
    throw new Error(
      `${directory}/run.json must record a supported engine and its argv for --${mode}`,
    )
  const results = readJson(path.join(directory, 'results.json'))
  return { directory: path.resolve(directory), metadata, results }
}

export function main(args = process.argv.slice(2)) {
  const { values } = parseArgs({
    args,
    options: {
      neo: { type: 'string' },
      reference: { type: 'string' },
      bridge: { type: 'string' },
      out: { type: 'string' },
      json: { type: 'string' },
      help: { type: 'boolean', short: 'h' },
    },
  })
  if (values.help) {
    console.log(
      'node report.mjs --neo <dir> --reference <dir> --bridge <cases.jsonl> --out <report.md> [--json <verdicts.jsonl>]',
    )
    return
  }
  for (const key of ['neo', 'reference', 'bridge', 'out'])
    if (!values[key]) throw new Error(`Missing required --${key}`)
  const inputs = [
    values.bridge,
    fileURLToPath(new URL('../scope.json', import.meta.url)),
    ...[values.neo, values.reference].flatMap((dir) =>
      ['results.json', 'run.json', 'scope.json'].map((file) =>
        path.join(dir, file),
      ),
    ),
  ].map((file) => path.resolve(file))
  const outputs = [values.out, values.json]
    .filter(Boolean)
    .map((file) => path.resolve(file))
  if (
    new Set(outputs).size !== outputs.length ||
    outputs.some((file) => inputs.includes(file))
  )
    throw new Error(
      'Output paths must be distinct and must not overwrite an input',
    )
  const neo = readRun(values.neo, 'neo')
  const reference = readRun(values.reference, 'reference')
  const bridge = fs
    .readFileSync(values.bridge, 'utf8')
    .split(/\r?\n/)
    .filter((line) => line.trim())
    .map((line, index) => {
      try {
        return JSON.parse(line)
      } catch (error) {
        throw new Error(`${values.bridge}:${index + 1}: ${error.message}`)
      }
    })
  const corpus = readJson(
    fileURLToPath(new URL('../../corpus.json', import.meta.url)),
  ).suites
  const report = renderReport({ neo, reference, bridge, corpus })
  for (const file of outputs)
    fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(values.out, report.markdown)
  if (values.json)
    fs.writeFileSync(
      values.json,
      report.rows.map((row) => JSON.stringify(row)).join('\n') +
        (report.rows.length ? '\n' : ''),
    )
  console.log(
    JSON.stringify({
      cases: report.rows.length,
      counts: report.counts,
      scores: report.scores,
      report: path.resolve(values.out),
      ...(values.json ? { json: path.resolve(values.json) } : {}),
    }),
  )
  return report
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  try {
    main()
  } catch (error) {
    console.error(`report: ${error.message}`)
    process.exitCode = 1
  }
}
