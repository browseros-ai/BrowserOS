/**
 * The CI gate around a finished `bun run test:neo-spec --ci` run: holds it to the platform's
 * committed list, reruns what one run cannot settle, and writes the verdict, the job summary and
 * the list the run supports; and `--expected-from`, which takes a CI run's list into the checkout.
 * Explicit decisions in the run's scope manifest are also enforced as negative tests, apart
 * from the positive expected list, so a passing refusal cannot be accepted by updating that list.
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
  knownFlakyFile,
}: {
  runner: Runner
  output: string
  complete: boolean
  writeExpected: boolean
  /** The committed list to hold the run to; tests pass their own instead of the platform's. */
  listFile?: string
  /** known-flaky.tsv to read; tests pass their own. */
  knownFlakyFile?: string
}): Promise<number> {
  const expected = await import('./node/expected.mjs')
  const { loadScope } = await import('./node/scope.mjs')
  const { judgeDecisions, decisionErrors } = await import(
    './node/decisions.mjs'
  )
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
  const scope = loadScope(path.join(output, 'scope.json'))
  const tests = expected.inScopeTests(report, scope)
  // Positive expectations omit excluded tests. Decisions are a separate, unwaivable
  // negative contract over the same raw report, including cases on known-flaky.tsv.
  const decisions = judgeDecisions(report, scope, { complete })
  const { readKnownFlaky } = await import('./node/known-flaky.mjs')
  const knownFlaky = readKnownFlaky(knownFlakyFile)
  const verdict = expected.compare(tests, listed, { complete, knownFlaky })
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
  const next = expected.nextExpected(tests, listed, verdict, {
    complete,
    knownFlaky,
  })
  const errors = [
    ...runErrors(report, run),
    // A rerun that broke the harness leaves its test's result unproven, even when another
    // attempt matched: a browser that survived cleanup can disturb the reruns after it.
    ...rerunErrors,
    ...decisionErrors(decisions),
  ]
  // The update command lets a test leave the list only with this evidence from the same job;
  // the job's errors travel with it, so a broken run proves no pass.
  const proof = expected.proofOf(tests, listed, next, verdict, {
    knownFlaky,
    errors,
  })
  fs.mkdirSync(path.join(output, 'expected'), { recursive: true })
  fs.writeFileSync(
    path.join(output, 'expected', `${platform}.tsv`),
    expected.formatExpected(next, platform),
  )
  fs.writeFileSync(
    path.join(output, 'expected', `${platform}.proof.json`),
    JSON.stringify(proof, null, 2),
  )
  const actions = process.env.GITHUB_ACTIONS === 'true'
  const markdown = expected.summaryMarkdown({
    verdict,
    scores,
    committed,
    next,
    decisions,
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
        decisions,
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
    const refused = expected.refusals(proof, next, { knownFlaky })
    if (refused.length) {
      console.error(
        `Not writing ${listFile}, a test would leave it without proof:\n${refused.map((reason: string) => `  ${reason}`).join('\n')}`,
      )
      return 1
    }
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
 * the list stays generated by the runner on the platform it describes. `download` and `mainList`
 * are the GitHub CLI and git by default; tests pass their own.
 */
export async function pullExpected(
  run: string,
  {
    download = ghDownload,
    ...adopt
  }: {
    download?: (
      id: string,
      repo: string | undefined,
      dir: string,
    ) => void | Promise<void>
  } & AdoptOptions = {},
): Promise<number> {
  const id = /(?:^|\/runs\/)(\d+)(?:[/?#]|$)/.exec(run)?.[1]
  if (!id)
    throw new Error(
      `--expected-from takes a GitHub Actions run ID or URL, not "${run}"`,
    )
  const repo = /github\.com\/([^/]+\/[^/]+)\/actions\/runs\//.exec(run)?.[1]
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-spec-expected-'))
  try {
    await download(id, repo, scratch)
    // Awaited here: the finally below deletes the download.
    return await adoptLists(scratch, adopt)
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true })
  }
}

async function ghDownload(id: string, repo: string | undefined, dir: string) {
  const { ARTIFACTS } = await import('./node/expected.mjs')
  const download = spawnSync(
    'gh',
    [
      'run',
      'download',
      id,
      ...(repo ? ['--repo', repo] : []),
      '--name',
      ARTIFACTS.expected,
      '--dir',
      dir,
    ],
    { stdio: 'inherit' },
  )
  if (download.error || download.status !== 0)
    throw new Error(
      `Could not download ${ARTIFACTS.expected} from run ${id}${download.error ? ` (${download.error.message})` : ''}. It needs the GitHub CLI, signed in (gh auth login).`,
    )
}

type AdoptOptions = {
  /** Where a platform's list is committed. */
  target?: (platform: string) => string
  knownFlakyFile?: string
  /** origin/main's list for a platform, or undefined; git by default. */
  mainList?: (
    platform: string,
    file: string,
  ) => Map<string, unknown> | string | undefined
}

/** origin/main's copy of a list, after a best-effort fetch, so main's own removals are recognized. */
function originMainList(_platform: string, file: string) {
  const cwd = path.dirname(file)
  spawnSync('git', ['fetch', '--quiet', 'origin', 'main'], {
    cwd,
    stdio: 'ignore',
  })
  const top = spawnSync('git', ['rev-parse', '--show-toplevel'], {
    cwd,
    encoding: 'utf8',
  })
  const relative = path.relative(top.stdout.trim(), file)
  const shown = spawnSync('git', ['show', `origin/main:${relative}`], {
    cwd,
    encoding: 'utf8',
  })
  return shown.status === 0 ? shown.stdout : undefined
}
/**
 * Why a new list may not replace `current` (the checkout's), one line per test it would drop
 * without proof. A test the run judged needs that run's proof (`refusals`). A test the run never
 * judged, because its list already lacked it, may go only if origin/main's list lacks it too:
 * main dropped it with a proof of its own. Anything else would let an artifact erase entries.
 */
function dropsWithoutProof(
  proof: unknown,
  after: Map<string, unknown>,
  current: Map<string, { id: string | null; file: string; title: string }>,
  main: Map<string, unknown> | undefined,
  knownFlaky: Map<string, unknown>,
  expected: typeof import('./node/expected.mjs'),
): string[] {
  const judged = new Set(
    ((proof as { base?: { file: string; title: string }[] }).base ?? []).map(
      (entry) => expected.testKey(entry.file, entry.title),
    ),
  )
  const reasons = expected.refusals(proof, after, { knownFlaky })
  for (const [key, entry] of current) {
    if (after.has(key) || judged.has(key) || (main && !main.has(key))) continue
    reasons.push(
      `${entry.id ?? '-'} ${entry.file} › ${entry.title}: the run did not judge it (its list lacked it) and origin/main still lists it; take a run of this branch merged with the current main`,
    )
  }
  return reasons
}

/**
 * Takes the lists a CI run wrote (each `<platform>.tsv` with its `<platform>.proof.json`) into
 * the checkout, all or none: a list that would drop a test without proof is refused, and the
 * command says which test and why. Returns the exit code.
 */
export async function adoptLists(
  directory: string,
  { target, knownFlakyFile, mainList }: AdoptOptions = {},
): Promise<number> {
  const expected = await import('./node/expected.mjs')
  const { readKnownFlaky } = await import('./node/known-flaky.mjs')
  const knownFlaky = readKnownFlaky(knownFlakyFile)
  const names = fs
    .readdirSync(directory)
    .filter((name) => /^[a-z0-9]+-[a-z0-9_]+\.tsv$/.test(name))
  if (!names.length) throw new Error(`${directory} holds no list`)
  const lists = names.map((name) => {
    const platform = name.slice(0, -'.tsv'.length)
    const file = (target ?? expected.expectedPath)(platform)
    const text = fs.readFileSync(path.join(directory, name), 'utf8')
    const after = expected.parseExpected(text)
    const current = expected.readExpected(file) ?? new Map()
    const proofFile = path.join(directory, `${platform}.proof.json`)
    if (!fs.existsSync(proofFile))
      return {
        platform,
        file,
        text,
        after,
        current,
        refused: [
          `the run has no ${platform}.proof.json: it predates proofs, so rerun its job`,
        ],
      }
    const shown = (mainList ?? originMainList)(platform, file)
    const main =
      typeof shown === 'string' ? expected.parseExpected(shown) : shown
    const proof = JSON.parse(fs.readFileSync(proofFile, 'utf8'))
    const refused = dropsWithoutProof(
      proof,
      after,
      current,
      main,
      knownFlaky,
      expected,
    )
    return { platform, file, text, after, current, refused }
  })
  const refused = lists.flatMap(({ platform, refused }) =>
    refused.map((reason: string) => `  ${platform}: ${reason}`),
  )
  if (refused.length) {
    console.error(
      `Not taking this run's lists: a test leaves the expected list only with proof.\n${refused.join('\n')}`,
    )
    return 1
  }
  for (const { file, text, after, current } of lists) {
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, text)
    console.log(`${file}: ${expected.describeChange(current, after)}`)
  }
  return 0
}
