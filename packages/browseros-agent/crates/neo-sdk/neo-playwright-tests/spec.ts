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
/** run.mjs and Playwright flags whose value is the next argument, so it is not a spec name. */
const VALUE_FLAGS = new Set([
  '--grep',
  '-g',
  '--grep-invert',
  '-G',
  '--workers',
  '-j',
  '--engine',
  '--test-list',
  '--test-list-invert',
  '--repeat-each',
  '--retries',
  '--timeout',
  '--max-failures',
  '--shard',
])

/** Splits the command line into spec names and the flags passed through to run.mjs. */
export function splitArgs(argv: string[]): {
  names: string[]
  flags: string[]
} {
  const names: string[] = []
  const flags: string[] = []
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index]
    if (arg === '--') {
      names.push(...argv.slice(index + 1))
      break
    }
    if (!arg.startsWith('-')) names.push(arg)
    else if (VALUE_FLAGS.has(arg) && index + 1 < argv.length)
      flags.push(arg, argv[++index])
    else flags.push(arg)
  }
  return { names, flags }
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

/**
 * NEO_TESTS_PLAYWRIGHT_SOURCE when set; else a cache folder per pinned release, fetched at the
 * pinned tag and prepared by js/prepare.mjs the first time. The stamp prepare.mjs writes last
 * marks a finished folder, so an interrupted preparation is redone, not trusted.
 */
function ensureSource(node: string, env: NodeJS.ProcessEnv): string {
  if (env.NEO_TESTS_PLAYWRIGHT_SOURCE) return env.NEO_TESTS_PLAYWRIGHT_SOURCE
  const cache = path.join(
    env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'),
    'browseros',
    'neo-spec',
  )
  const source = path.join(cache, `playwright-${corpus.version}`)
  if (fs.existsSync(path.join(source, '.neo-source.json'))) return source
  console.log(
    `Preparing Playwright v${corpus.version} test sources in ${source} (first run only: clone, npm ci, build; a few minutes).`,
  )
  fs.mkdirSync(cache, { recursive: true })
  const checkout = `${source}.checkout`
  const log = path.join(cache, `prepare-${corpus.version}.log`)
  fs.rmSync(source, { recursive: true, force: true })
  fs.rmSync(checkout, { recursive: true, force: true })
  const out = fs.openSync(log, 'w')
  const step = (command: string, args: string[], cwd = cache) => {
    const result = spawnSync(command, args, {
      cwd,
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
    step(node, [path.join(crate, 'js/prepare.mjs'), checkout], crate)
  } finally {
    fs.closeSync(out)
    fs.rmSync(checkout, { recursive: true, force: true })
  }
  return source
}

/** The command that reruns the first in-scope failure (or any failure) of a finished run. */
async function rerunCommand(output: string, flags: string[]): Promise<string> {
  const { flattenResults, outcome } = await import('./node/report/classify.mjs')
  const { excludedBy } = await import('./node/scope.mjs')
  const scope = JSON.parse(
    fs.readFileSync(path.join(output, 'scope.json'), 'utf8'),
  )
  const failures = flattenResults(
    JSON.parse(fs.readFileSync(path.join(output, 'results.json'), 'utf8')),
    corpus.suites,
  ).filter((row) => outcome(row) === 'fail')
  const row =
    failures.find((failure) => !excludedBy(scope, failure)) ?? failures[0]
  if (!row) return 'Nothing failed.'
  const outOfScope = excludedBy(scope, row)?.reason
  const quote = (text: string) => `'${text.replaceAll("'", `'\\''`)}'`
  const title = row.titles.at(-1).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const { runnerOptions } = await import('./node/options.mjs')
  const engine = runnerOptions(flags).reference ? ' --reference' : ''
  // The shortest name that resolves back to this one spec.
  const folderName = row.suite.replace(/^tests\/|\.spec\.ts$/g, '')
  const fileName = path.basename(folderName)
  const included = corpus.suites.filter((suite) => suite.include)
  const name =
    included.filter(
      (suite) => path.basename(suite.file, '.spec.ts') === fileName,
    ).length === 1
      ? fileName
      : folderName
  const label = `${row.caseId ?? 'one failure'}${outOfScope ? ` (out of scope: ${outOfScope})` : ''}`
  return `Rerun ${label}: bun run test:neo-spec ${name}${engine} --grep ${quote(title)}`
}

async function main(argv: string[]): Promise<number> {
  const { names, flags } = splitArgs(argv)
  const files = resolveSuites(
    names,
    corpus.suites.filter((suite) => suite.include).map((suite) => suite.file),
  )
  const node = findNode()
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    // npm (for preparing the source) and run.mjs's own children use this same Node.
    PATH: [path.dirname(node), process.env.PATH].join(path.delimiter),
    NEO_E2E_BROWSER: findBrowser(),
  }
  env.NEO_TESTS_PLAYWRIGHT_SOURCE = ensureSource(node, env)
  const resultsFile = path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), 'neo-spec-')),
    'results-dir',
  )
  // The child owns Ctrl-C: run.mjs stops Playwright and cleans up its browsers, then exits.
  const ignore = () => {}
  process.on('SIGINT', ignore)
  process.on('SIGTERM', ignore)
  const code = await new Promise<number>((resolve) => {
    spawn(node, [path.join(crate, 'node/run.mjs'), ...flags, ...files], {
      stdio: 'inherit',
      env: { ...env, NEO_NODE_RESULTS_FILE: resultsFile },
    }).once('exit', (status, signal) => resolve(status ?? (signal ? 1 : 0)))
  })
  process.off('SIGINT', ignore)
  process.off('SIGTERM', ignore)
  const output = fs.existsSync(resultsFile)
    ? fs.readFileSync(resultsFile, 'utf8')
    : undefined
  fs.rmSync(path.dirname(resultsFile), { recursive: true, force: true })
  const scoresFile = output && path.join(output, 'scores.json')
  if (!scoresFile || !fs.existsSync(scoresFile)) {
    console.error(
      `\nNo scores: the run stopped before Playwright finished.${output ? ` Results: ${output}` : ''}`,
    )
    return code || 1
  }
  const { scoreText } = await import('./node/scope.mjs')
  const scores = JSON.parse(fs.readFileSync(scoresFile, 'utf8'))
  console.log(
    [
      '',
      `In scope:  ${scoreText(scores.inScope)}`,
      `All tests: ${scoreText(scores.all)}`,
      `Results:   ${output}`,
      await rerunCommand(output, flags),
    ].join('\n'),
  )
  return code
}

if (import.meta.main)
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (error) => {
      console.error(error.message)
      process.exit(1)
    },
  )
