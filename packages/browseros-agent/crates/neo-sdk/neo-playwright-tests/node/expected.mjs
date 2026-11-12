/**
 * Neo: The spec suite as a gate. Each platform commits `expected/<platform>.tsv`, the in-scope
 * tests that do not pass there, written by the runner and never by hand. `bun run test:neo-spec
 * --ci` runs the whole corpus and passes only when the run reproduces that list: a test outside
 * it that does not pass is a regression, a listed test that passes must leave the list in the same
 * pull request, so the in-scope score only moves up.
 *
 *   full run ─▶ compare(tests, list) ─▶ settle: rerun the unsettled ones alone ─▶ verdict
 *                                                                                ├─▶ summary.md
 *                                                                                └─▶ the list this run supports
 *
 * scope.json alone decides which tests count; nothing here changes selection, scope or curation.
 */
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { stripVTControlCharacters } from 'node:util'
import { flattenResults, outcome } from './report/classify.mjs'
import { excludedBy } from './scope.mjs'

/**
 * The CI job's artifact names: neo-spec.yml uploads the results folder and the list this run
 * supports under them, and `--expected-from` downloads the second.
 */
export const ARTIFACTS = {
  report: 'neo-spec-report',
  expected: 'neo-spec-expected',
}
/** A failing test is rerun alone at most this often before its verdict stands. */
export const RERUN_ATTEMPTS = 2
const OUTCOMES = ['fail', 'skip']
const COLUMNS = 'id\toutcome\tfile\ttitle'

/** `linux-x64`: which committed list a run on this machine is held to. */
export function platformKey(platform = process.platform, arch = process.arch) {
  return `${platform}-${arch}`
}

export function expectedPath(platform = platformKey()) {
  return fileURLToPath(new URL(`./expected/${platform}.tsv`, import.meta.url))
}

/**
 * Tests are keyed by spec file and title path, not by case ID: a parameterized test shares one
 * ID across its instances (expect-builtins' EX-088 has twelve).
 */
export const testKey = (file, title) => `${file}\t${title}`

/** One line per test, sorted, so two pull requests that each fix a test merge without conflict. */
export function formatExpected(entries, platform) {
  const lines = [...entries.values()]
    .sort(
      (a, b) =>
        a.file.localeCompare(b.file, 'en') ||
        a.title.localeCompare(b.title, 'en'),
    )
    .map((entry) => {
      for (const field of [entry.file, entry.title])
        if (/[\t\r\n]/.test(field))
          throw new Error(`A tab or newline in ${JSON.stringify(field)}`)
      return [entry.id ?? '-', entry.outcome, entry.file, entry.title].join(
        '\t',
      )
    })
  return [
    `# In-scope Playwright spec tests that do not pass on ${platform}: fail, or skip (ran nothing).`,
    '# Written by `bun run test:neo-spec --ci`; do not edit by hand. A pull request whose CI run',
    "# reports a changed test takes that run's list: `bun run test:neo-spec --expected-from <run URL>`.",
    `# ${COLUMNS}`,
    ...lines,
    '',
  ].join('\n')
}

export function parseExpected(text) {
  const entries = new Map()
  text.split('\n').forEach((line, index) => {
    if (!line || line.startsWith('#')) return
    const fields = line.split('\t')
    const [id, result, file, title] = fields
    if (
      fields.length !== 4 ||
      !OUTCOMES.includes(result) ||
      !/^tests\/(page|library)\/[^/]+\.spec\.ts$/.test(file) ||
      !title
    )
      throw new Error(
        `Expected list line ${index + 1} is not "${COLUMNS.replaceAll('\t', '<tab>')}"`,
      )
    const key = testKey(file, title)
    if (entries.has(key))
      throw new Error(`Expected list names ${file} › ${title} twice`)
    entries.set(key, {
      id: id === '-' ? null : id,
      outcome: result,
      file,
      title,
    })
  })
  return entries
}

/** The committed list, or undefined when this platform has none yet. */
export function readExpected(file) {
  return fs.existsSync(file)
    ? parseExpected(fs.readFileSync(file, 'utf8'))
    : undefined
}

function firstError(test) {
  const result = test.results?.at(-1)
  const error = result?.error ?? result?.errors?.[0]
  const text = stripVTControlCharacters(String(error?.message ?? ''))
  return text.trim().split('\n')[0].slice(0, 300) || undefined
}

/**
 * The in-scope tests of a Playwright JSON report, each pass, skip or fail. Fail covers failed,
 * timed out, interrupted and never reported: none of them passed.
 */
export function inScopeTests(report, scope) {
  const tests = new Map()
  for (const row of flattenResults(report)) {
    if (excludedBy(scope, row)) continue
    const result = outcome(row)
    const key = testKey(row.suite, row.title)
    tests.set(key, {
      key,
      id: row.caseId,
      file: row.suite,
      title: row.title,
      titles: row.titles,
      outcome: result === 'pass' || result === 'skip' ? result : 'fail',
      error:
        result === 'pass' || result === 'skip'
          ? undefined
          : firstError(row.test),
    })
  }
  return tests
}

/**
 * The verdict one full run can give. `rerun` holds what it cannot settle by itself: an expected
 * pass that failed (a regression, or flaky) and a listed test that passed (fixed, or flaky the
 * other way). A skip is decided by the spec or the harness, not by timing, so it is never rerun.
 * `complete` says the run selected the whole corpus, so a listed test it lacks is stale.
 */
export function compare(tests, expected, { complete }) {
  const verdict = {
    regressions: [],
    newlyPassing: [],
    flaky: [],
    changed: [],
    stale: [],
    inconclusive: [],
    rerun: [],
  }
  for (const test of tests.values()) {
    const listed = expected.get(test.key)
    if (!listed) {
      if (test.outcome === 'fail') verdict.rerun.push({ test })
      else if (test.outcome === 'skip')
        verdict.regressions.push({ test, reruns: [] })
    } else if (test.outcome === 'pass') verdict.rerun.push({ test, listed })
    // fail <-> skip: still not passing, but a newly skipped test leaves the rate's denominator,
    // so the list records the change and review sees it.
    else if (test.outcome !== listed.outcome)
      verdict.changed.push({ test, listed })
  }
  if (complete)
    for (const [key, entry] of expected)
      if (!tests.has(key)) verdict.stale.push(entry)
  return verdict
}

/** Rerun outcomes that are a test's own; anything else (`not-run`, `error`) says the rerun broke. */
const TEST_OUTCOMES = ['pass', 'fail', 'skip']

/**
 * Reruns every unsettled test alone, `concurrency` at a time; `rerun(test, attempt)` resolves to
 * `{ outcome }`. An expected pass that passes a rerun is flaky, one that fails all
 * RERUN_ATTEMPTS is a regression. A listed test is newly passing only if it passes every rerun as
 * well; one that fails a rerun is flaky and stays listed, so the list never drops a test that
 * still fails sometimes. A rerun that broke (no result for the test) proves neither, so a test
 * left without a verdict is inconclusive, which fails the run instead of passing it as flaky.
 */
export async function settle(
  verdict,
  rerun,
  { attempts = RERUN_ATTEMPTS, concurrency = 4 } = {},
) {
  const queue = [...verdict.rerun]
  const work = async () => {
    for (let item = queue.shift(); item; item = queue.shift()) {
      const reruns = []
      while (reruns.length < attempts) {
        let result
        try {
          result = await rerun(item.test, reruns.length + 1)
        } catch (error) {
          result = { outcome: 'error', error: error.message }
        }
        reruns.push(result)
        const passed = result.outcome === 'pass'
        if (item.listed ? !passed : passed) break
      }
      const count = (test) => reruns.filter((result) => test(result.outcome))
      const passes = count((outcome) => outcome === 'pass').length
      const failures = count(
        (outcome) => outcome !== 'pass' && TEST_OUTCOMES.includes(outcome),
      ).length
      const broken = reruns.length - passes - failures
      const settled = item.listed
        ? failures
          ? verdict.flaky
          : broken
            ? verdict.inconclusive
            : verdict.newlyPassing
        : passes
          ? verdict.flaky
          : broken
            ? verdict.inconclusive
            : verdict.regressions
      settled.push({ ...item, reruns })
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(concurrency, queue.length) }, work),
  )
  verdict.rerun = []
  const order = (a, b) =>
    (a.test ?? a).file.localeCompare((b.test ?? b).file, 'en') ||
    (a.test ?? a).title.localeCompare((b.test ?? b).title, 'en')
  for (const list of Object.values(verdict)) list.sort(order)
  return verdict
}

export const passed = (verdict) =>
  ['regressions', 'newlyPassing', 'changed', 'stale', 'inconclusive'].every(
    (name) => !verdict[name].length,
  )

/**
 * The list this run supports: the committed one, less the tests that now pass and the entries
 * for tests no longer in the corpus, plus the regressions, each with this run's outcome. A
 * flaky listed test keeps its entry. A filtered run keeps the entries it did not select.
 */
export function nextExpected(tests, expected, verdict, { complete }) {
  const dropped = new Set(verdict.newlyPassing.map((item) => item.test.key))
  const added = new Set(verdict.regressions.map((item) => item.test.key))
  const next = new Map()
  if (!complete)
    for (const [key, entry] of expected)
      if (!tests.has(key)) next.set(key, entry)
  for (const test of tests.values()) {
    const listed = expected.get(test.key)
    if (!(listed && !dropped.has(test.key)) && !added.has(test.key)) continue
    next.set(test.key, {
      id: test.id,
      outcome: test.outcome === 'pass' ? listed.outcome : test.outcome,
      file: test.file,
      title: test.title,
    })
  }
  return next
}

/** What `--write-expected` or `--expected-from` changes, for the person committing it. */
export function describeChange(before, after) {
  const added = [...after.keys()].filter((key) => !before.has(key))
  const removed = [...before.keys()].filter((key) => !after.has(key))
  const changed = [...after.keys()].filter(
    (key) =>
      before.has(key) && before.get(key).outcome !== after.get(key).outcome,
  )
  const name = (entry) => `${entry.id ?? '-'} ${entry.file} › ${entry.title}`
  return [
    `${before.size} → ${after.size} tests listed: ${removed.length} removed, ${added.length} added, ${changed.length} changed outcome.`,
    ...removed.map((key) => `  - ${name(before.get(key))}`),
    ...added.map(
      (key) =>
        `  + ${name(after.get(key))} (${after.get(key).outcome}; adding a test accepts a regression, so say why in the pull request)`,
    ),
    ...changed.map(
      (key) =>
        `  ~ ${name(after.get(key))} (${before.get(key).outcome} → ${after.get(key).outcome})`,
    ),
  ].join('\n')
}

const markdown = (text) => String(text).replace(/[\\`*_[\]<>|#]/g, '\\$&')
const seconds = (ms) => {
  const total = Math.round(ms / 1000)
  return total >= 60
    ? `${Math.floor(total / 60)} min ${total % 60} s`
    : `${total} s`
}
const rate = (score) =>
  score.total ? `${((100 * score.passed) / score.total).toFixed(1)}%` : 'n/a'

/**
 * The job summary: totals, then every test the verdict names. `run` carries what the caller
 * knows about the run (platform, workers, wall times, the CI run's URL and artifact names).
 */
export function summaryMarkdown({ verdict, scores, run, committed, next }) {
  const name = (test) =>
    `${test.id ? `**${test.id}** ` : ''}${markdown(test.file)} › ${markdown(test.title)}`
  const reruns = (item) =>
    item.reruns.length
      ? `alone: ${item.reruns.map((result) => result.outcome).join(', ')}`
      : 'not rerun (a skip does not depend on timing)'
  const errorOf = (item) =>
    item.test.error ? ` — ${markdown(item.test.error)}` : ''
  const update = run.url
    ? `\`bun run test:neo-spec --expected-from ${run.url}\``
    : '`bun run test:neo-spec --ci --write-expected` on this platform'
  const ok = passed(verdict) && committed && !run.errors?.length
  const counts = [
    [verdict.regressions.length, 'regression', 'regressions'],
    [verdict.newlyPassing.length, 'newly passing', 'newly passing'],
    [verdict.changed.length, 'changed outcome', 'changed outcomes'],
    [verdict.stale.length, 'stale entry', 'stale entries'],
    [verdict.inconclusive.length, 'inconclusive', 'inconclusive'],
  ]
    .filter(([count]) => count)
    .map(([count, one, many]) => `${count} ${count === 1 ? one : many}`)
  const lines = [
    `## Neo spec suite, ${run.platform}: ${ok ? '✅ matches its expected list' : `❌ ${[...(committed ? counts : ['no committed expected list']), ...(run.errors?.length ? ['run error'] : [])].join(', ')}`}`,
    '',
    '| Tests | Passed | Counted | Rate | Skipped (not counted) |',
    '|---|--:|--:|--:|--:|',
    `| In scope | ${scores.inScope.passed} | ${scores.inScope.total} | ${rate(scores.inScope)} | ${scores.inScope.skipped} (${scores.inScope.skippedByHarness} by the harness) |`,
    `| All | ${scores.all.passed} | ${scores.all.total} | ${rate(scores.all)} | ${scores.all.skipped} |`,
    '',
    `Expected not to pass: ${committed ? committed.size : 'no list committed'}; this run supports ${next.size}. Flaky: ${verdict.flaky.length}.`,
    `Suite ${seconds(run.suiteMs)} on ${run.workers} workers${run.rerunCount ? `; ${run.rerunCount} reruns alone in ${seconds(run.rerunMs)}` : ''}.${run.browser ? ` Browser: ${markdown(run.browser)}.` : ''}`,
  ]
  if (!committed)
    lines.push(
      '',
      `No \`expected/${run.platform}.tsv\` is committed, so every test that did not pass counts as a regression below. Commit this run's list: ${update}.`,
    )
  for (const error of run.errors ?? [])
    lines.push('', `**The run reported an error:** ${markdown(error)}`)
  const section = (title, intro, items, line) => {
    if (!items.length) return
    lines.push(
      '',
      `### ${title} (${items.length})`,
      '',
      intro,
      '',
      ...items.map(line),
    )
  }
  section(
    'Regressions',
    'Expected to pass; did not pass in the full run or in any rerun alone.',
    verdict.regressions,
    (item) =>
      `- ${name(item.test)}: ${item.test.outcome}, ${reruns(item)}${errorOf(item)}`,
  )
  section(
    'Newly passing',
    `Listed as not passing; passed in the full run and in every rerun alone. Take them off the list in this pull request: ${update}.`,
    verdict.newlyPassing,
    (item) => `- ${name(item.test)} (was ${item.listed.outcome})`,
  )
  section(
    'Changed outcome',
    `Still not passing, but no longer as listed. Update the list: ${update}.`,
    verdict.changed,
    (item) =>
      `- ${name(item.test)}: ${item.listed.outcome} → ${item.test.outcome}`,
  )
  section(
    'Stale entries',
    `Listed, but not in the corpus any more. Update the list: ${update}.`,
    verdict.stale,
    (entry) => `- ${name(entry)}`,
  )
  section(
    'Inconclusive',
    'A rerun alone produced no result for the test (see its log in the report), so neither its full-run result nor the list can be confirmed.',
    verdict.inconclusive,
    (item) =>
      `- ${name(item.test)}: ${item.listed ? 'listed' : 'expected to pass'}; full run ${item.test.outcome}, ${item.reruns.map((result) => `${result.outcome}${result.note ? ` (${markdown(result.note)})` : ''}`).join(', ')}`,
  )
  section(
    'Flaky',
    'Their full-run result differed from a rerun alone; not counted against this run.',
    verdict.flaky,
    (item) =>
      `- ${name(item.test)}: ${item.listed ? 'listed' : 'expected to pass'}; full run ${item.test.outcome}, ${reruns(item)}${errorOf(item)}`,
  )
  if (run.artifacts)
    lines.push(
      '',
      `Full report: artifact \`${run.artifacts.report}\`. The list this run supports: artifact \`${run.artifacts.expected}\`.`,
    )
  return `${lines.join('\n')}\n`
}
