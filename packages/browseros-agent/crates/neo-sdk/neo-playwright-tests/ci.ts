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
): Promise<{ outcome: string; output?: string; log: string; note?: string }> {
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
  const results = output && path.join(output, 'results.json')
  if (!results || !fs.existsSync(results))
    return { outcome: 'not-run', output, log, note: 'no results' }
  const rows = flattenResults(JSON.parse(fs.readFileSync(results, 'utf8')))
  if (rows.length !== 1 || testKey(rows[0].suite, rows[0].title) !== test.key)
    return {
      outcome: 'not-run',
      output,
      log,
      note: `the rerun selected ${rows.length} tests`,
    }
  const result = outcome(rows[0])
  return {
    outcome: result === 'pass' || result === 'skip' ? result : 'fail',
    output,
    log,
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
}: {
  runner: Runner
  output: string
  complete: boolean
  writeExpected: boolean
}): Promise<number> {
  const expected = await import('./node/expected.mjs')
  const { loadScope } = await import('./node/scope.mjs')
  const read = (file: string) =>
    JSON.parse(fs.readFileSync(path.join(output, file), 'utf8'))
  const report = read('results.json')
  const scores = read('scores.json')
  const run = read('run.json')
  const platform = expected.platformKey()
  const listFile = expected.expectedPath(platform)
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
  if (verdict.rerun.length)
    console.log(
      `\nRerunning ${verdict.rerun.length} tests alone, up to ${expected.RERUN_ATTEMPTS} times each.`,
    )
  await expected.settle(verdict, async (test: CiTest, attempt: number) => {
    rerunCount++
    const result = await rerunAlone(
      runner,
      test,
      attempt,
      path.join(output, 'reruns'),
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
    ...(report.errors ?? []).map(
      (error: { message?: string }) =>
        stripVTControlCharacters(String(error.message ?? error)).split('\n')[0],
    ),
    ...(run.signal ? [`the run was stopped by ${run.signal}`] : []),
    // run.mjs's own failures: a launcher error, or browsers it owned that survived cleanup.
    ...[run.error, run.cleanupError]
      .filter(Boolean)
      .map((error: string) => String(error).split('\n')[0]),
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
