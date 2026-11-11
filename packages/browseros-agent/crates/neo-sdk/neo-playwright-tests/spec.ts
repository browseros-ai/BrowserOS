/**
 * `bun run test:neo-spec [names] [--reference] [run.mjs flags]`: Playwright's own spec tests
 * against Neo, with no setup. It does only what a person did by hand before `node/run.mjs`:
 * find a Node that Playwright Test runs on, find the BrowserOS neo executable, and prepare the
 * pinned Playwright test source once into a cache. Engines, curation, scope and scores stay in
 * run.mjs; this file ends by printing the scores run.mjs recorded.
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

type LockOptions = {
  pid?: number
  isAlive?: (pid: number) => boolean
  wait?: () => void
  onWait?: () => void
}

/**
 * The pid in `lock`: a number, `undefined` while a new owner has not written it yet, or `null`
 * once the lock is gone (released between two reads).
 */
function lockOwner(lock: string): number | undefined | null {
  try {
    return Number(fs.readFileSync(path.join(lock, 'pid'), 'utf8')) || undefined
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
    return fs.existsSync(lock) ? undefined : null
  }
}

/** A lock whose owner died, or that got no owner within a minute of being made. */
function isStale(
  lock: string,
  owner: number | undefined,
  isAlive: (pid: number) => boolean,
): boolean {
  if (owner) return !isAlive(owner)
  try {
    return Date.now() - fs.statSync(lock).mtimeMs > 60_000
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
    return false
  }
}

/**
 * Removes `lock` if its owner died, and says whether to try for it again at once. Two waiting
 * runs can both see the dead owner, so the removal happens under `<lock>.takeover`, after
 * reading the owner again there: a run acting on what it saw earlier cannot delete the lock
 * another run has just taken. Every removal goes through the guard, so that reread is final.
 */
function reclaim(lock: string, isAlive: (pid: number) => boolean): boolean {
  const owner = lockOwner(lock)
  if (owner === null) return true
  if (!isStale(lock, owner, isAlive)) return false
  const guard = `${lock}.takeover`
  try {
    fs.mkdirSync(guard)
  } catch (error) {
    if (error.code !== 'EEXIST') throw error
    // The guard is held for a few file operations; one this old belongs to a run that died.
    if (isStale(guard, undefined, isAlive))
      fs.rmSync(guard, { recursive: true, force: true })
    return false
  }
  try {
    const current = lockOwner(lock)
    if (current === owner && isStale(lock, current, isAlive))
      fs.rmSync(lock, { recursive: true, force: true })
    return true
  } finally {
    fs.rmSync(guard, { recursive: true, force: true })
  }
}

/**
 * Takes `lock` (a directory, so taking it is one atomic mkdir) for this process, waiting while
 * a live process holds it and taking over from one that died. It keeps two first runs from
 * preparing, and deleting, the same cache folder at once.
 */
export function acquireLock(
  lock: string,
  {
    pid = process.pid,
    isAlive = processAlive,
    wait = () => Bun.sleepSync(2_000),
    onWait = () => {},
  }: LockOptions = {},
) {
  for (let waited = false; ; ) {
    try {
      fs.mkdirSync(lock)
      fs.writeFileSync(path.join(lock, 'pid'), String(pid))
      return
    } catch (error) {
      if (error.code !== 'EEXIST') throw error
    }
    if (reclaim(lock, isAlive)) continue
    if (!waited) onWait()
    waited = true
    wait()
  }
}

/** Releases `lock` if `pid` still holds it. */
export function releaseLock(lock: string, pid = process.pid) {
  if (lockOwner(lock) === pid) fs.rmSync(lock, { recursive: true, force: true })
}

/** Shallow-clones the pinned tag and prepares it into `source` with js/prepare.mjs. */
function prepareSource(node: string, env: NodeJS.ProcessEnv, source: string) {
  const checkout = `${source}.checkout`
  const log = `${source}.prepare.log`
  fs.rmSync(source, { recursive: true, force: true })
  fs.rmSync(checkout, { recursive: true, force: true })
  const out = fs.openSync(log, 'w')
  const step = (command: string, args: string[]) => {
    const result = spawnSync(command, args, {
      cwd: crate,
      env: { ...env, NEO_TESTS_PLAYWRIGHT_SOURCE: source },
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
}

/**
 * NEO_TESTS_PLAYWRIGHT_SOURCE when set; else a folder per pinned release in the cache
 * (NEO_SPEC_CACHE_DIR, default ~/.cache/browseros/neo-spec), prepared the first time. The stamp prepare.mjs writes last marks a finished folder, so an interrupted
 * preparation is redone, not trusted.
 */
function ensureSource(node: string, env: NodeJS.ProcessEnv): string {
  if (env.NEO_TESTS_PLAYWRIGHT_SOURCE) return env.NEO_TESTS_PLAYWRIGHT_SOURCE
  const cache =
    env.NEO_SPEC_CACHE_DIR ||
    path.join(
      env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'),
      'browseros',
      'neo-spec',
    )
  const source = path.join(cache, `playwright-${corpus.version}`)
  const ready = () => fs.existsSync(path.join(source, '.neo-source.json'))
  if (ready()) return source
  fs.mkdirSync(cache, { recursive: true })
  const lock = `${source}.lock`
  acquireLock(lock, {
    onWait: () => console.log(`Waiting for another run to prepare ${source}.`),
  })
  try {
    if (!ready()) {
      console.log(
        `Preparing Playwright v${corpus.version} test sources in ${source} (first run only: clone, npm ci, build; a few minutes).`,
      )
      prepareSource(node, env, source)
    }
  } finally {
    releaseLock(lock)
  }
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
): Promise<{ code: number; output?: string }> {
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-spec-'))
  const resultsFile = path.join(scratch, 'results-dir')
  const child = spawn(node, [path.join(crate, 'node/run.mjs'), ...args], {
    stdio: 'inherit',
    env: { ...env, NEO_NODE_RESULTS_FILE: resultsFile },
  })
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
  const failed = pick(rows.filter((row) => outcome(row) === 'fail'))
  const row = failed ?? pick(rows.filter(cutOff))
  const stopped =
    signal || report.errors?.length || rows.some(cutOff)
      ? `Stopped${signal ? ` by ${signal}` : ''} before every test ran.`
      : ''
  if (!row)
    return stopped ? `${stopped} Nothing had failed.` : 'Nothing failed.'
  const outOfScope = excludedBy(scope, row)?.reason
  const escapeRegex = (text: string) =>
    text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const quote = (text: string) =>
    /^[\w./=:@%+,-]+$/.test(text) ? text : `'${text.replaceAll("'", `'\\''`)}'`
  // Playwright greps "<project> <file> <title path> <tags>"; anchoring both ends keeps a short
  // title from also selecting longer titles that start with it (curation.mjs does the same).
  const grep = `(?:^| )${escapeRegex(row.suite.replace(/^tests\//, ''))} ${escapeRegex(row.titles.join(' '))}(?: @\\S+)*$`
  // The shortest name that resolves back to this one spec.
  const folderName = row.suite.replace(/^tests\/|\.spec\.ts$/g, '')
  const fileName = path.basename(folderName)
  const sameName = corpus.suites.filter(
    (suite) =>
      suite.include && path.basename(suite.file, '.spec.ts') === fileName,
  )
  const name = sameName.length === 1 ? fileName : folderName
  const label = `${row.caseId ?? 'one test'}${failed ? '' : ' (interrupted)'}${outOfScope ? ` (out of scope: ${outOfScope})` : ''}`
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

async function main(argv: string[]): Promise<number> {
  const { names, flags } = splitArgs(argv)
  const files = resolveSuites(
    names,
    corpus.suites.filter((suite) => suite.include).map((suite) => suite.file),
  )
  const node = findNode()
  const env = runnerEnv({ node, flags })
  env.NEO_TESTS_PLAYWRIGHT_SOURCE = ensureSource(node, env)
  const { code, output } = await runRunner(node, env, [...flags, ...files])
  return finish(code, output, flags)
}

if (import.meta.main)
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (error) => {
      console.error(error.message)
      process.exit(1)
    },
  )
