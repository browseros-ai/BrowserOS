/**
 * The CI gate around a finished `bun run test:neo-spec --ci` run: holds it to the platform's
 * committed list, reruns what one run cannot settle, and writes the verdict, the job summary and
 * the list the run supports; and `--expected-from`, which takes a CI run's list into the checkout.
 * The rules live in node/expected.mjs; this file runs tests and moves files. spec.ts owns the
 * command line and hands in how to start run.mjs.
 */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { stripVTControlCharacters } from 'node:util'

/** How spec.ts starts run.mjs, and the arguments of the full run. */
export type Runner = {
  node: string
  env: NodeJS.ProcessEnv
  flags: string[]
  runRunner: (
    node: string,
    env: NodeJS.ProcessEnv,
    args: string[],
    log?: string,
  ) => Promise<{ code: number; output?: string }>
  rerunArgs: (
    flags: string[],
    test: { file: string; titles: string[] },
  ) => string[]
}

/**
 * What went wrong with a run.mjs run itself, apart from test failures: Playwright's top-level
 * errors, a stop signal, a launcher error, browsers it owned that survived cleanup, or a runner
 * that never finished. run.mjs writes `endedAt` in its last `finally`, so a run.json without it
 * belongs to a runner that was killed; its exit code alone cannot say so, because a signal
 * reaches spec.ts as exit 1, the same code as a failing test.
 */
function runErrors(
  report: { errors?: { message?: string }[] } | undefined,
  run:
    | {
        signal?: string
        error?: string
        cleanupError?: string
        endedAt?: string
      }
    | undefined,
): string[] {
  const firstLine = (text: unknown) =>
    stripVTControlCharacters(String(text)).split('\n')[0]
  return [
    ...(report?.errors ?? []).map((error) => firstLine(error.message ?? error)),
    ...(run?.signal ? [`the run was stopped by ${run.signal}`] : []),
    ...[run?.error, run?.cleanupError].filter(Boolean).map(firstLine),
    ...(run && !run.endedAt
      ? ['the runner did not finish (run.json has no endedAt)']
      : []),
  ]
}

type CiTest = {
  key: string
  id: string | null
  file: string
  title: string
  titles: string[]
}

/** Reruns one test alone and reads back its outcome; anything but one matching result is not a pass. */
async function rerunAlone(
  { node, env, flags, runRunner, rerunArgs }: Runner,
  test: CiTest,
  attempt: number,
  directory: string,
): Promise<{
  outcome: string
  output?: string
  log: string
  note?: string
  errors: string[]
}> {
  const { flattenResults, outcome } = await import('./node/report/classify.mjs')
  const { testKey } = await import('./node/expected.mjs')
  fs.mkdirSync(directory, { recursive: true })
  const tag = createHash('sha256').update(test.key).digest('hex').slice(0, 8)
  const log = path.join(directory, `${test.id ?? 'test'}-${tag}-${attempt}.log`)
  const { output } = await runRunner(
    node,
    { ...env, NEO_NODE_OUTPUT_DIR: directory },
    rerunArgs(flags, test),
    log,
  )
  // run.json first: a report that cannot be read must not hide the runner's own errors.
  const errors: string[] = []
  const read = (file: string) => {
    const target = output && path.join(output, file)
    if (!target || !fs.existsSync(target)) return undefined
    try {
      return JSON.parse(fs.readFileSync(target, 'utf8'))
    } catch (error) {
      errors.push(`${file} is not JSON: ${String(error)}`)
      return undefined
    }
  }
  const run = read('run.json')
  const report = read('results.json')
  errors.push(...runErrors(report, run))
  if (!report)
    return { outcome: 'not-run', output, log, note: 'no results', errors }
  let rows: ReturnType<typeof flattenResults>
  try {
    rows = flattenResults(report)
  } catch (error) {
    errors.push(`results.json cannot be read: ${String(error)}`)
    return {
      outcome: 'not-run',
      output,
      log,
      note: 'unreadable results',
      errors,
    }
  }
  if (rows.length !== 1 || testKey(rows[0].suite, rows[0].title) !== test.key)
    return {
      outcome: 'not-run',
      output,
      log,
      note: `the rerun selected ${rows.length} tests`,
      errors,
    }
  const result = outcome(rows[0])
  return {
    outcome: result === 'pass' || result === 'skip' ? result : 'fail',
    output,
    log,
    errors,
  }
}

/**
 * The CI verdict for a finished run: compares it with the committed list, reruns what it cannot
 * settle, and writes verdict.json, summary.md (also the GitHub job summary) and
 * expected/<platform>.tsv, the list this run supports, into the results folder. Returns the exit
 * code: 0 only when the run reproduces a committed list, up to flaky tests.
 */
export async function ciVerdict({
  runner,
  output,
  complete,
  writeExpected,
  listFile: listOverride,
}: {
  runner: Runner
  output: string
  complete: boolean
  writeExpected: boolean
  /** The committed list to hold the run to; tests pass their own instead of the platform's. */
  listFile?: string
}): Promise<number> {
  const expected = await import('./node/expected.mjs')
  const { loadScope } = await import('./node/scope.mjs')
  const read = (file: string) =>
    JSON.parse(fs.readFileSync(path.join(output, file), 'utf8'))
  const report = read('results.json')
  const scores = read('scores.json')
  const run = read('run.json')
  const platform = expected.platformKey()
  const listFile = listOverride ?? expected.expectedPath(platform)
  const committed = expected.readExpected(listFile)
  const listed = committed ?? new Map()
  // The run's own copy of scope.json: the policy it was scored with.
  const tests = expected.inScopeTests(
    report,
    loadScope(path.join(output, 'scope.json')),
  )
  const verdict = expected.compare(tests, listed, { complete })
  const rerunStarted = Date.now()
  let rerunCount = 0
  const rerunErrors: string[] = []
  if (verdict.rerun.length)
    console.log(
      `\nRerunning ${verdict.rerun.length} tests alone, up to ${expected.RERUN_ATTEMPTS} times each.`,
    )
  await expected.settle(verdict, async (test: CiTest, attempt: number) => {
    rerunCount++
    const which = `rerun ${attempt} of ${test.id ?? test.title}`
    let result: Awaited<ReturnType<typeof rerunAlone>>
    try {
      result = await rerunAlone(
        runner,
        test,
        attempt,
        path.join(output, 'reruns'),
      )
    } catch (error) {
      // settle() records the attempt as broken; the error itself fails the verdict here.
      rerunErrors.push(`${which}: ${String(error)}`)
      throw error
    }
    for (const error of result.errors)
      rerunErrors.push(`${which}: ${error} (log ${result.log})`)
    if (!['pass', 'fail', 'skip'].includes(result.outcome))
      rerunErrors.push(
        `${which} produced no result for the test: ${result.note} (log ${result.log})`,
      )
    console.log(
      `  ${result.outcome.padEnd(7)} ${test.id ?? '-'} ${test.file} › ${test.title} (attempt ${attempt}${result.note ? `; ${result.note}` : ''}; log ${result.log})`,
    )
    return result
  })
  const next = expected.nextExpected(tests, listed, verdict, { complete })
  fs.mkdirSync(path.join(output, 'expected'), { recursive: true })
  fs.writeFileSync(
    path.join(output, 'expected', `${platform}.tsv`),
    expected.formatExpected(next, platform),
  )
  const errors = [
    ...runErrors(report, run),
    // A rerun that broke the harness leaves its test's result unproven, even when another
    // attempt matched: a browser that survived cleanup can disturb the reruns after it.
    ...rerunErrors,
  ]
  const actions = process.env.GITHUB_ACTIONS === 'true'
  const markdown = expected.summaryMarkdown({
    verdict,
    scores,
    committed,
    next,
    run: {
      platform,
      workers: run.workers,
      suiteMs: report.stats?.duration ?? 0,
      rerunCount,
      rerunMs: Date.now() - rerunStarted,
      browser: process.env.NEO_SPEC_BROWSER_LABEL,
      errors,
      url:
        actions && process.env.GITHUB_RUN_ID
          ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`
          : undefined,
      artifacts: actions ? expected.ARTIFACTS : undefined,
    },
  })
  const ok = expected.passed(verdict) && committed && !errors.length
  fs.writeFileSync(
    path.join(output, 'verdict.json'),
    JSON.stringify(
      {
        passed: Boolean(ok),
        platform,
        complete,
        committed: !!committed,
        ...verdict,
      },
      null,
      2,
    ),
  )
  fs.writeFileSync(path.join(output, 'summary.md'), markdown)
  if (process.env.GITHUB_STEP_SUMMARY)
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, markdown)
  console.log(`\n${markdown}`)
  if (writeExpected) {
    fs.mkdirSync(path.dirname(listFile), { recursive: true })
    fs.writeFileSync(listFile, expected.formatExpected(next, platform))
    console.log(`Wrote ${listFile}: ${expected.describeChange(listed, next)}`)
    return errors.length ? 1 : 0
  }
  return ok ? 0 : 1
}

/**
 * Replaces the committed lists with those of a finished CI run (its `neo-spec-expected`
 * artifact), through the GitHub CLI. This is how a pull request takes the list its CI run wrote:
 * the list stays generated by the runner on the platform it describes.
 */
export async function pullExpected(run: string): Promise<number> {
  const expected = await import('./node/expected.mjs')
  const id = /(?:^|\/runs\/)(\d+)(?:[/?#]|$)/.exec(run)?.[1]
  if (!id)
    throw new Error(
      `--expected-from takes a GitHub Actions run ID or URL, not "${run}"`,
    )
  const repo = /github\.com\/([^/]+\/[^/]+)\/actions\/runs\//.exec(run)?.[1]
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-spec-expected-'))
  try {
    const download = spawnSync(
      'gh',
      [
        'run',
        'download',
        id,
        ...(repo ? ['--repo', repo] : []),
        '--name',
        expected.ARTIFACTS.expected,
        '--dir',
        scratch,
      ],
      { stdio: 'inherit' },
    )
    if (download.error || download.status !== 0)
      throw new Error(
        `Could not download ${expected.ARTIFACTS.expected} from run ${id}${download.error ? ` (${download.error.message})` : ''}. It needs the GitHub CLI, signed in (gh auth login).`,
      )
    const lists = fs
      .readdirSync(scratch)
      .filter((name) => /^[a-z0-9]+-[a-z0-9_]+\.tsv$/.test(name))
    if (!lists.length)
      throw new Error(
        `Run ${id}'s ${expected.ARTIFACTS.expected} holds no list`,
      )
    for (const name of lists) {
      const text = fs.readFileSync(path.join(scratch, name), 'utf8')
      const after = expected.parseExpected(text)
      const target = expected.expectedPath(name.slice(0, -'.tsv'.length))
      const before = expected.readExpected(target) ?? new Map()
      fs.mkdirSync(path.dirname(target), { recursive: true })
      fs.writeFileSync(target, text)
      console.log(`${target}: ${expected.describeChange(before, after)}`)
    }
    return 0
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true })
  }
}
