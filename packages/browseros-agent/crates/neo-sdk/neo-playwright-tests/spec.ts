/**
 * `bun run test:neo-spec [names] [--reference] [run.mjs flags]`: Playwright's own spec tests
 * against Neo, with no setup. It does only what a person did by hand before `node/run.mjs`:
 * find a Node that Playwright Test runs on, find the BrowserOS neo executable, and prepare the
 * pinned Playwright test source once into a cache. Engines, curation, scope and scores stay in
 * run.mjs; this file ends by printing the scores run.mjs recorded.
 *
 * `--ci` makes the same run a pass/fail gate (CI's neo-spec.yml calls exactly this): it holds the
 * run to the platform's committed list of tests that do not pass, reruns what one run cannot
 * settle, and writes the verdict, a job summary and the list the run supports
 * (node/expected.mjs). `--expected-from <run>` takes a CI run's list into the checkout.
 */
import { spawn, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const crate = path.dirname(fileURLToPath(import.meta.url))
const corpus: {
  revision: string
  version: string
  suites: { file: string; include: boolean }[]
} = JSON.parse(fs.readFileSync(path.join(crate, 'corpus.json'), 'utf8'))

/**
 * Playwright 1.64.1 needs Node 20; the harness itself uses `Promise.withResolvers` (Node 22).
 * The corpus has been run on Node 26, the version Homebrew's fallback below provides.
 */
const MIN_NODE_MAJOR = 22
const NODE_FALLBACKS = [
  '/opt/homebrew/opt/node@26/bin',
  '/usr/local/opt/node@26/bin',
]
const DEFAULT_BROWSERS: Partial<Record<NodeJS.Platform, string>> = {
  darwin: '/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo',
  // The Linux .deb (package `browserclaw`) puts the browser on PATH as `browserclaw`.
  linux: '/usr/bin/browserclaw',
}
/**
 * Flags whose value is the next argument, so it is not read as a spec name: run.mjs's
 * `--engine` and Playwright 1.64.1's `test` options that take a required value. Options with
 * an optional value (`--only-changed`, `--shuffle`) need the `--flag=value` form.
 */
const VALUE_FLAGS = new Set([
  '--engine',
  '--add-reporter',
  '-g',
  '--grep',
  '-G',
  '--grep-invert',
  '--global-timeout',
  '-j',
  '--workers',
  '--last-failed-file',
  '--max-failures',
  '--project',
  '--repeat-each',
  '--retries',
  '--run-agents',
  '--shard',
  '--test-list',
  '--test-list-invert',
  '--timeout',
  '--trace',
  '--tsconfig',
  '--update-source-method',
])
/** Flags that choose tests; a rerun of one test replaces them with its own `--grep`. */
const SELECTION_FLAGS = new Set([
  '-g',
  '--grep',
  '-G',
  '--grep-invert',
  '--last-failed',
  '--last-failed-file',
  '--only-changed',
  '--shard',
  '--test-list',
  '--test-list-invert',
])

/** The command line as spec names and flags, each flag with its value if it takes one. */
function parseArgs(argv: string[]): { names: string[]; flags: string[][] } {
  const names: string[] = []
  const flags: string[][] = []
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index]
    if (arg === '--') {
      names.push(...argv.slice(index + 1))
      break
    }
    if (!arg.startsWith('-')) names.push(arg)
    else if (VALUE_FLAGS.has(arg) && index + 1 < argv.length)
      flags.push([arg, argv[++index]])
    else flags.push([arg])
  }
  return { names, flags }
}

/** Splits the command line into spec names and the flags passed through to run.mjs. */
export function splitArgs(argv: string[]): {
  names: string[]
  flags: string[]
} {
  const { names, flags } = parseArgs(argv)
  return { names, flags: flags.flat() }
}

/** The run's flags without the ones that chose its tests, for rerunning a single test. */
export function rerunFlags(flags: string[]): string[] {
  return parseArgs(flags)
    .flags.filter(([flag]) => !SELECTION_FLAGS.has(flag.split('=')[0]))
    .flat()
}

/**
 * Turns `page-goto`, `page/page-goto`, `page-goto.spec.ts` or `tests/page/page-goto.spec.ts`
 * into the corpus path. run.mjs hands its filters to Playwright as path patterns, where a bare
 * `page-click` would also select page-click-react, page-click-scroll and the rest.
 */
export function resolveSuites(names: string[], suites: string[]): string[] {
  const short = (file: string) =>
    file.replace(/^(?:.*\/)?tests\//, '').replace(/\.spec\.ts$/, '')
  return names.map((name) => {
    const wanted = short(name)
    const matches = suites.filter(
      (file) => short(file) === wanted || path.basename(short(file)) === wanted,
    )
    if (matches.length === 1) return matches[0]
    if (matches.length > 1)
      throw new Error(
        `"${name}" names ${matches.length} specs (${matches.join(', ')}); give its folder, e.g. ${short(matches[0])}`,
      )
    const near = suites
      .filter((file) => short(file).includes(wanted))
      .slice(0, 5)
      .map(short)
    throw new Error(
      `No spec named "${name}" in the corpus.${near.length ? ` Close: ${near.join(', ')}.` : ''} Names are spec files without .spec.ts, listed in corpus.json.`,
    )
  })
}

type NodeProbe = { node: string; bun?: string } | undefined

function probeNode(executable: string): NodeProbe {
  const result = spawnSync(
    executable,
    ['-p', 'JSON.stringify(process.versions)'],
    { encoding: 'utf8', timeout: 10_000 },
  )
  if (result.status !== 0) return undefined
  try {
    return JSON.parse(result.stdout)
  } catch {
    return undefined
  }
}

/**
 * The first real Node, at least MIN_NODE_MAJOR, on PATH and then in the fallbacks. `bun run`
 * can put Bun's own `node` stand-in on PATH; Playwright Test does not run on it.
 */
export function findNode({
  pathEnv = process.env.PATH ?? '',
  fallbacks = NODE_FALLBACKS,
  probe = probeNode,
  exists = fs.existsSync,
}: {
  pathEnv?: string
  fallbacks?: string[]
  probe?: (executable: string) => NodeProbe
  exists?: (file: string) => boolean
} = {}): string {
  const rejected: string[] = []
  const directories = [...pathEnv.split(path.delimiter), ...fallbacks]
  for (const directory of new Set(directories.filter(Boolean))) {
    const executable = path.join(directory, 'node')
    if (!exists(executable)) continue
    const versions = probe(executable)
    if (!versions) rejected.push(`${executable} (does not run)`)
    else if (versions.bun) rejected.push(`${executable} (Bun's node stand-in)`)
    else if (Number(versions.node.split('.')[0]) < MIN_NODE_MAJOR)
      rejected.push(`${executable} (Node ${versions.node})`)
    else return executable
  }
  throw new Error(
    [
      `No Node.js ${MIN_NODE_MAJOR} or newer found on PATH or in ${fallbacks.join(', ')}.`,
      ...(rejected.length ? [`Not usable: ${rejected.join('; ')}.`] : []),
      'Install it with `brew install node@26` (macOS) or from https://nodejs.org, then run this again.',
    ].join('\n'),
  )
}

/** NEO_E2E_BROWSER when set, else the install path BrowserOS neo uses on this OS. */
export function findBrowser({
  env = process.env,
  platform = process.platform,
  exists = fs.existsSync,
}: {
  env?: Record<string, string | undefined>
  platform?: NodeJS.Platform
  exists?: (file: string) => boolean
} = {}): string {
  const chosen = env.NEO_E2E_BROWSER
  if (chosen) {
    if (exists(chosen)) return chosen
    throw new Error(`NEO_E2E_BROWSER is ${chosen}, which does not exist.`)
  }
  const installed = DEFAULT_BROWSERS[platform]
  if (installed && exists(installed)) return installed
  throw new Error(
    [
      installed
        ? `BrowserOS neo is not installed at ${installed}.`
        : `No default BrowserOS neo location on ${platform}.`,
      'Install it (macOS: https://cdn.browseros.com/download/BrowserOS_neo.dmg) or set NEO_E2E_BROWSER to its executable.',
    ].join('\n'),
  )
}

const processAlive = (pid: number) => {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return error.code === 'EPERM'
  }
}

const isReady = (source: string) =>
  fs.existsSync(path.join(source, '.neo-source.json'))

/** The prepared source this run uses, as an absolute path (git and npm run in other folders). */
export function sourceDir(env: NodeJS.ProcessEnv): string {
  if (env.NEO_TESTS_PLAYWRIGHT_SOURCE)
    return path.resolve(env.NEO_TESTS_PLAYWRIGHT_SOURCE)
  const cache =
    env.NEO_SPEC_CACHE_DIR ||
    path.join(
      env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'),
      'browseros',
      'neo-spec',
    )
  return path.resolve(cache, `playwright-${corpus.version}`)
}

/**
 * Prepares into a private `<source>.staging-<pid>` folder and publishes it with one rename, so
 * `source` only ever appears complete. Two first runs each prepare their own copy; the first
 * rename wins and the other discards only its own staging. No run ever deletes a folder another
 * run is preparing or has published, which a shared lock could not promise once its holder died.
 */
export function publishSource(
  source: string,
  build: (staging: string) => void,
  pid = process.pid,
) {
  const staging = `${source}.staging-${pid}`
  // Same pid, earlier run: its staging, clone and log belong to a run that died, and the sweep
  // skips this pid. git clone refuses a destination that already exists.
  for (const leftover of [staging, `${staging}.checkout`, `${staging}.log`])
    fs.rmSync(leftover, { recursive: true, force: true })
  try {
    build(staging)
    try {
      fs.renameSync(staging, source)
    } catch (error) {
      // A rename onto a non-empty folder fails: another run published first.
      if (!['ENOTEMPTY', 'EEXIST'].includes(error.code) || !isReady(source))
        throw error
    }
  } finally {
    fs.rmSync(staging, { recursive: true, force: true })
  }
}

/** Removes staging folders, checkouts and logs that runs which died left beside `source`. */
export function sweepStaging(source: string, isAlive = processAlive) {
  const name = path.basename(source)
  const pattern = new RegExp(
    `^${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\.staging-(\\d+)(?:\\.checkout|\\.log)?$`,
  )
  for (const entry of fs.readdirSync(path.dirname(source))) {
    const owner = Number(pattern.exec(entry)?.[1])
    if (owner && owner !== process.pid && !isAlive(owner))
      fs.rmSync(path.join(path.dirname(source), entry), {
        recursive: true,
        force: true,
      })
  }
}

/** Shallow-clones the pinned tag and prepares it into `target` with js/prepare.mjs. */
function buildSource(node: string, env: NodeJS.ProcessEnv, target: string) {
  const checkout = `${target}.checkout`
  const log = `${target}.log`
  const out = fs.openSync(log, 'w')
  const step = (command: string, args: string[]) => {
    const result = spawnSync(command, args, {
      cwd: crate,
      env: { ...env, NEO_TESTS_PLAYWRIGHT_SOURCE: target },
      stdio: ['ignore', out, out],
    })
    if (result.error || result.status !== 0)
      throw new Error(
        `Preparing the Playwright source failed at \`${command} ${args.join(' ')}\`; see ${log}`,
      )
  }
  try {
    step('git', [
      'clone',
      '--quiet',
      '--depth=1',
      `--branch=v${corpus.version}`,
      'https://github.com/microsoft/playwright.git',
      checkout,
    ])
    const head = spawnSync('git', ['-C', checkout, 'rev-parse', 'HEAD'], {
      encoding: 'utf8',
    }).stdout.trim()
    if (head !== corpus.revision)
      throw new Error(
        `Playwright's tag v${corpus.version} is ${head}, but corpus.json pins ${corpus.revision}`,
      )
    step(node, [path.join(crate, 'js/prepare.mjs'), checkout])
  } finally {
    fs.closeSync(out)
    fs.rmSync(checkout, { recursive: true, force: true })
  }
  fs.rmSync(log, { force: true })
}

/**
 * NEO_TESTS_PLAYWRIGHT_SOURCE when set; else a folder per pinned release in the cache
 * (NEO_SPEC_CACHE_DIR, default ~/.cache/browseros/neo-spec), prepared the first time.
 */
export function ensureSource(
  node: string,
  env: NodeJS.ProcessEnv,
  {
    build = (staging: string) => buildSource(node, env, staging),
    isAlive = processAlive,
  }: {
    build?: (staging: string) => void
    isAlive?: (pid: number) => boolean
  } = {},
): string {
  const source = sourceDir(env)
  if (env.NEO_TESTS_PLAYWRIGHT_SOURCE) return source
  fs.mkdirSync(path.dirname(source), { recursive: true })
  // Every run sweeps: a run that died after another published leaves a large staging folder.
  sweepStaging(source, isAlive)
  if (isReady(source)) return source
  if (fs.existsSync(source)) {
    // Another run's rename may have landed between the two reads; it publishes complete folders.
    if (isReady(source)) return source
    throw new Error(
      `${source} is incomplete: an earlier version of this command stopped while preparing it. Delete it and run again.`,
    )
  }
  console.log(
    `Preparing Playwright v${corpus.version} test sources in ${source} (first run only: clone, npm ci, build; a few minutes).`,
  )
  publishSource(source, build)
  return source
}

/**
 * Runs node/run.mjs on the terminal and returns its exit code and results folder. The terminal
 * delivers Ctrl-C to run.mjs itself, which stops Playwright and cleans up its browsers, so the
 * wrapper only waits for it; a SIGTERM sent to the wrapper alone is passed on.
 */
async function runRunner(
  node: string,
  env: NodeJS.ProcessEnv,
  args: string[],
  log?: string,
): Promise<{ code: number; output?: string }> {
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-spec-'))
  const resultsFile = path.join(scratch, 'results-dir')
  // Reruns run several at a time, so each writes its own log instead of the terminal.
  const out = log === undefined ? undefined : fs.openSync(log, 'w')
  const child = spawn(node, [path.join(crate, 'node/run.mjs'), ...args], {
    stdio: out === undefined ? 'inherit' : ['ignore', out, out],
    env: { ...env, NEO_NODE_RESULTS_FILE: resultsFile },
  })
  if (out !== undefined) fs.closeSync(out)
  const wait = () => {}
  const forward = () => child.kill('SIGTERM')
  process.on('SIGINT', wait)
  process.on('SIGTERM', forward)
  try {
    const code = await new Promise<number>((resolve) =>
      child.once('exit', (status, signal) =>
        resolve(status ?? (signal ? 1 : 0)),
      ),
    )
    const output = fs.existsSync(resultsFile)
      ? fs.readFileSync(resultsFile, 'utf8')
      : undefined
    return { code, output }
  } finally {
    process.off('SIGINT', wait)
    process.off('SIGTERM', forward)
    fs.rmSync(scratch, { recursive: true, force: true })
  }
}

const escapeRegex = (text: string) =>
  text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/**
 * A `--grep` that selects exactly one test. Playwright greps "<project> <file> <title path>
 * <tags>"; anchoring both ends keeps a short title from also selecting longer titles that start
 * with it (curation.mjs does the same).
 */
export function exactGrep(suite: string, titles: string[]): string {
  return `(?:^| )${escapeRegex(suite.replace(/^tests\//, ''))} ${escapeRegex(titles.join(' '))}(?: @\\S+)*$`
}

/**
 * The last closing line: the command that reruns one test. It picks a failure (in scope
 * first), else a test the run cut off. A stopped run says so first: Playwright reports the test
 * it interrupted, and every test it never started, as skipped, so the scores alone look clean.
 */
export async function rerunLine(
  output: string,
  flags: string[],
): Promise<string> {
  const { flattenResults, outcome } = await import('./node/report/classify.mjs')
  const { excludedBy } = await import('./node/scope.mjs')
  const read = (file: string) =>
    JSON.parse(fs.readFileSync(path.join(output, file), 'utf8'))
  const scope = read('scope.json')
  const report = read('results.json')
  const signal = fs.existsSync(path.join(output, 'run.json'))
    ? read('run.json').signal
    : undefined
  // `--repeat-each` repeats a case; any failing instance names the test.
  type Row = ReturnType<typeof flattenResults>[number]
  const rows: Row[] = flattenResults(report, corpus.suites, {
    allowRepeated: true,
  })
  const cutOff = (row: Row) =>
    outcome(row) === 'harness-error' ||
    (row.test.results ?? []).some(
      (result: { status: string }) => result.status === 'interrupted',
    )
  const pick = (candidates: Row[]) =>
    candidates.find((row) => !excludedBy(scope, row)) ?? candidates[0]
  // A test the run never reached: skipped with no attempt and no skip or fixme of its own.
  const notRun = (row: Row) =>
    row.test.status === 'skipped' &&
    !row.test.results?.length &&
    !(row.test.annotations ?? []).some((annotation: { type: string }) =>
      ['skip', 'fixme'].includes(annotation.type),
    )
  const failed = pick(rows.filter((row) => outcome(row) === 'fail'))
  const stopped =
    signal || report.errors?.length || rows.some(cutOff)
      ? `Stopped${signal ? ` by ${signal}` : ''} before every test ran.`
      : ''
  const interrupted = failed ? undefined : pick(rows.filter(cutOff))
  const row =
    failed ?? interrupted ?? (stopped ? pick(rows.filter(notRun)) : undefined)
  if (!row)
    return stopped ? `${stopped} Nothing had failed.` : 'Nothing failed.'
  const outOfScope = excludedBy(scope, row)?.reason
  const quote = (text: string) =>
    /^[\w./=:@%+,-]+$/.test(text) ? text : `'${text.replaceAll("'", `'\\''`)}'`
  const grep = exactGrep(row.suite, row.titles)
  // The shortest name that resolves back to this one spec.
  const folderName = row.suite.replace(/^tests\/|\.spec\.ts$/g, '')
  const fileName = path.basename(folderName)
  const sameName = corpus.suites.filter(
    (suite) =>
      suite.include && path.basename(suite.file, '.spec.ts') === fileName,
  )
  const name = sameName.length === 1 ? fileName : folderName
  const status = failed ? '' : interrupted ? ' (interrupted)' : ' (not run)'
  const label = `${row.caseId ?? 'one test'}${status}${outOfScope ? ` (out of scope: ${outOfScope})` : ''}`
  return `${stopped ? `${stopped} ` : ''}Rerun ${label}: ${['bun run test:neo-spec', name, ...rerunFlags(flags).map(quote), '--grep', quote(grep)].join(' ')}`
}

/** The four closing lines, from the scores and results run.mjs recorded. */
async function summary(output: string, flags: string[]): Promise<string> {
  const { scoreText } = await import('./node/scope.mjs')
  const scores = JSON.parse(
    fs.readFileSync(path.join(output, 'scores.json'), 'utf8'),
  )
  return [
    '',
    `In scope:  ${scoreText(scores.inScope)}`,
    `All tests: ${scoreText(scores.all)}`,
    `Results:   ${output}`,
    await rerunLine(output, flags),
  ].join('\n')
}

/** The environment run.mjs needs: the chosen Node first on PATH and the browser to drive. */
export function runnerEnv({
  node,
  flags,
  env = process.env,
  browser = findBrowser,
}: {
  node: string
  flags: string[]
  env?: NodeJS.ProcessEnv
  browser?: () => string
}): NodeJS.ProcessEnv {
  return {
    ...env,
    // npm (for preparing the source) and run.mjs's own children use this same Node.
    PATH: [path.dirname(node), env.PATH].join(path.delimiter),
    // run.mjs lists tests without launching a browser.
    ...(flags.includes('--list') ? {} : { NEO_E2E_BROWSER: browser() }),
  }
}

/** Prints how the run ended and returns the wrapper's exit code. */
export async function finish(
  code: number,
  output: string | undefined,
  flags: string[],
  print: {
    log: (text: string) => void
    error: (text: string) => void
  } = console,
): Promise<number> {
  // A listing has no scores; Playwright's exit code says whether it worked.
  if (flags.includes('--list')) return code
  if (!output || !fs.existsSync(path.join(output, 'scores.json'))) {
    print.error(
      `\nNo scores: the run stopped before Playwright finished.${output ? ` Results: ${output}` : ''}`,
    )
    return code || 1
  }
  print.log(await summary(output, flags))
  return code
}

/** spec.ts's own switches, taken off the command line before the rest goes to run.mjs. */
export function ciArgs(argv: string[]): {
  ci: boolean
  writeExpected: boolean
  expectedFrom?: string
  argv: string[]
} {
  const rest: string[] = []
  let ci = false
  let writeExpected = false
  let expectedFrom: string | undefined
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index]
    if (arg === '--') {
      rest.push(...argv.slice(index))
      break
    }
    if (arg === '--ci') ci = true
    else if (arg === '--write-expected') writeExpected = true
    else if (arg === '--expected-from') expectedFrom = argv[++index] ?? ''
    else if (arg.startsWith('--expected-from='))
      expectedFrom = arg.slice('--expected-from='.length)
    else if (VALUE_FLAGS.has(arg) && index + 1 < argv.length)
      rest.push(arg, argv[++index])
    else rest.push(arg)
  }
  if (expectedFrom === '')
    throw new Error('--expected-from takes a GitHub Actions run ID or URL')
  if (expectedFrom !== undefined && (ci || rest.length))
    throw new Error(
      '--expected-from takes the list of a finished CI run and runs no tests; give it alone',
    )
  if (writeExpected && !ci) throw new Error('--write-expected goes with --ci')
  return { ci, writeExpected, expectedFrom, argv: rest }
}

/** Whether a run selected less than the whole corpus, so its verdict covers only its tests. */
export function isFiltered(names: string[], flags: string[]): boolean {
  return (
    names.length > 0 ||
    parseArgs(flags).flags.some(([flag]) =>
      SELECTION_FLAGS.has(flag.split('=')[0]),
    )
  )
}

/**
 * run.mjs's arguments for one test alone: the full run's flags without its selection and worker
 * count, one worker, and the add-on the full run built.
 */
export function ciRerunArgs(
  flags: string[],
  test: { file: string; titles: string[] },
): string[] {
  const kept = parseArgs(flags).flags.filter(([flag]) => {
    const name = flag.split('=')[0]
    return (
      !SELECTION_FLAGS.has(name) &&
      name !== '--workers' &&
      !name.startsWith('-j') &&
      name !== '--skip-build'
    )
  })
  return [
    ...kept.flat(),
    '--skip-build',
    '--workers',
    '1',
    test.file,
    '--grep',
    exactGrep(test.file, test.titles),
  ]
}

async function main(argv: string[]): Promise<number> {
  const own = ciArgs(argv)
  if (own.expectedFrom !== undefined)
    return (await import('./ci')).pullExpected(own.expectedFrom)
  const { names, flags } = splitArgs(own.argv)
  const filtered = isFiltered(names, flags)
  if (own.ci) {
    const { runnerOptions } = await import('./node/options.mjs')
    if (flags.includes('--list'))
      throw new Error('--ci runs tests; --list only lists them')
    if (runnerOptions(flags).engine !== 'claw')
      throw new Error("--ci holds Neo's claw engine to its expected list")
    if (own.writeExpected && filtered)
      throw new Error(
        '--write-expected needs the whole corpus: drop the spec names and --grep',
      )
  }
  const files = resolveSuites(
    names,
    corpus.suites.filter((suite) => suite.include).map((suite) => suite.file),
  )
  const node = findNode()
  const env = runnerEnv({ node, flags })
  env.NEO_TESTS_PLAYWRIGHT_SOURCE = ensureSource(node, env)
  const { code, output } = await runRunner(node, env, [...flags, ...files])
  // The workflow uploads this folder even when the verdict below throws.
  if (own.ci && output && process.env.GITHUB_OUTPUT)
    fs.appendFileSync(process.env.GITHUB_OUTPUT, `results=${output}\n`)
  const finished = await finish(code, output, flags)
  if (!own.ci || !output || !fs.existsSync(path.join(output, 'scores.json')))
    return finished
  return (await import('./ci')).ciVerdict({
    runner: { node, env, flags, runRunner, rerunArgs: ciRerunArgs },
    output,
    complete: !filtered,
    writeExpected: own.writeExpected,
  })
}

if (import.meta.main)
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (error) => {
      console.error(error.message)
      process.exit(1)
    },
  )
