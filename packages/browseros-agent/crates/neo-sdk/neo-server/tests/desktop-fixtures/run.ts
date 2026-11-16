/** Builds this checkout, then isolates density and terminal observations in launcher-owned browsers. */
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { constants, createReadStream } from 'node:fs'
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises'
import { constants as osConstants, tmpdir } from 'node:os'
import {
  basename,
  delimiter,
  dirname,
  isAbsolute,
  join,
  resolve,
} from 'node:path'
import { createInterface } from 'node:readline'
import { cases, validateInventory } from './inventory'
import { jsonObject, numberField, stringField } from './json'
import type { CaseResult, CaseSpec, ExecutableCase, JsonObject } from './model'
import { junit, type Partition, type PartitionReceipt } from './receipt'
import { readPendingSelect } from './terminal-select'

const AGENT_ROOT = resolve(import.meta.dir, '../../../../..')
const ADAPTER = join(import.meta.dir, 'browser-adapter.ts')
const SUITE = join(import.meta.dir, 'suite.ts')

interface RunOptions {
  check: boolean
  caseId?: string
  out: string
}

interface PartitionOutcome {
  partition: Partition
  startedAt: number
  startupMs: number
  casesMs: number
  cleanupMs: number
  finishedAt: number
  results: { id: string; status: 'pass' | 'xfail' | 'fail' }[]
  expectedIds: string[]
  complete: boolean
}

export function parseOptions(args: string[]): RunOptions {
  const options: RunOptions = {
    check: false,
    out: resolve(
      AGENT_ROOT,
      'target/sdk-desktop-fixtures',
      `run-${crypto.randomUUID()}`,
    ),
  }
  for (let index = 0; index < args.length; index++) {
    const arg = args[index]
    if (arg === '--check') options.check = true
    else if (arg === '--case' || arg === '--out') {
      const value = args[++index]
      if (!value || value.startsWith('--'))
        throw new Error(`${arg} requires a value`)
      if (arg === '--case') options.caseId = value
      else options.out = resolve(value)
    } else throw new Error(`Unknown option ${arg}`)
  }
  return options
}

export function partitionCases(
  selected: readonly ExecutableCase[],
  partition: Partition,
): readonly ExecutableCase[] {
  return selected.filter((entry) => entry.execution === partition)
}

/** Resolve the real browser before NEO_E2E_BROWSER is replaced with the executable adapter. */
export async function resolveBrowser(env: NodeJS.ProcessEnv): Promise<string> {
  const selected =
    env.BROWSEROS_BINARY ||
    env.NEO_E2E_BROWSER ||
    (process.platform === 'darwin'
      ? '/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo'
      : 'browserclaw')
  const candidates = selected.includes('/')
    ? [resolve(selected)]
    : (env.PATH ?? '')
        .split(delimiter)
        .map((directory) => resolve(directory, selected))
  for (const candidate of candidates) {
    try {
      await access(candidate, constants.X_OK)
      const binary = await realpath(candidate)
      if (binary === (await realpath(ADAPTER)))
        throw new Error('Browser adapter cannot launch itself')
      return binary
    } catch (error) {
      if (
        error instanceof Error &&
        error.message.includes('cannot launch itself')
      )
        throw error
    }
  }
  throw new Error(`Browser executable not found: ${selected}`)
}

function forwardSignals(child: ReturnType<typeof spawn>): () => void {
  const signals: NodeJS.Signals[] = ['SIGINT', 'SIGTERM', 'SIGHUP']
  const handlers = signals.map((signal) => {
    const handler = () => child.kill(signal)
    process.on(signal, handler)
    return { signal, handler }
  })
  return () => {
    for (const { signal, handler } of handlers) process.off(signal, handler)
  }
}

async function exited(child: ReturnType<typeof spawn>): Promise<number> {
  return await new Promise((resolveCode, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) =>
      resolveCode(code ?? (signal ? 128 + osConstants.signals[signal] : 125)),
    )
  })
}

async function sha256(file: string): Promise<string> {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(file)) hash.update(chunk)
  return hash.digest('hex')
}

/** Cargo's emitted executable is authoritative; neither a prebuilt override nor a guessed path is used. */
async function buildSource(out: string) {
  const buildDirectory = await mkdtemp(join(tmpdir(), 'sdk-desktop-build-'))
  const command = [
    'nice',
    '-n',
    '10',
    'cargo',
    'build',
    '--locked',
    '-p',
    'neo-server',
    '--bin',
    'browseros-claw-sdk',
    '--message-format=json-render-diagnostics',
  ]
  const startedAt = Date.now()
  let binary: string | undefined
  let parseFailure: unknown
  let failure: string | undefined
  try {
    const child = spawn('nice', command.slice(1), {
      cwd: AGENT_ROOT,
      env: {
        ...process.env,
        CARGO_BUILD_BUILD_DIR: buildDirectory,
        CARGO_PROFILE_DEV_DEBUG: '0',
        CARGO_BUILD_JOBS: '4',
      },
      stdio: ['ignore', 'pipe', 'inherit'],
    })
    const done = exited(child)
    const release = forwardSignals(child)
    try {
      for await (const line of createInterface({ input: child.stdout })) {
        try {
          const message = jsonObject(JSON.parse(line), 'Cargo build message')
          if (message.reason !== 'compiler-artifact') continue
          const target = jsonObject(message.target, 'Cargo build target')
          if (
            target.name === 'browseros-claw-sdk' &&
            typeof message.executable === 'string'
          )
            binary = message.executable
        } catch (error) {
          parseFailure = error
        }
      }
      const code = await done
      if (code !== 0) throw new Error(`Source SDK build failed (${code})`)
      if (parseFailure) throw parseFailure
      if (!binary || !isAbsolute(binary))
        throw new Error('Cargo did not report the source SDK executable')
      binary = await realpath(binary)
      await access(binary, constants.X_OK)
      const finishedAt = Date.now()
      return {
        command,
        binary,
        sha256: await sha256(binary),
        buildDirectory,
        startedAt,
        finishedAt,
        durationMs: finishedAt - startedAt,
      }
    } finally {
      release()
    }
  } catch (error) {
    failure = error instanceof Error ? error.message : String(error)
    throw error
  } finally {
    try {
      const finishedAt = Date.now()
      await writeFile(
        join(out, 'build.json'),
        `${JSON.stringify(
          {
            command,
            binary,
            buildDirectory,
            startedAt,
            finishedAt,
            durationMs: finishedAt - startedAt,
            complete: !failure,
            failure,
          },
          null,
          2,
        )}\n`,
        { mode: 0o600 },
      )
    } finally {
      await rm(buildDirectory, { recursive: true, force: true })
    }
  }
}

function strings(value: unknown, label: string): string[] {
  if (!Array.isArray(value)) throw new Error(`${label} must be an array`)
  return value.map((entry: unknown) => {
    if (typeof entry !== 'string')
      throw new Error(`${label} must contain strings`)
    return entry
  })
}

function duration(object: JsonObject, key: string): number {
  const value = numberField(object, key)
  if (!Number.isFinite(value) || value < 0)
    throw new Error(`${key} must be nonnegative`)
  return value
}

function partitionOutcome(value: unknown): PartitionOutcome {
  const object = jsonObject(value, 'partition receipt')
  const partition = stringField(object, 'partition')
  if (
    partition !== 'ordinary' &&
    partition !== 'density' &&
    partition !== 'terminal-select'
  )
    throw new Error('Invalid receipt partition')
  if (!Array.isArray(object.results))
    throw new Error('Missing partition results')
  const results = object.results.map(
    (entry): PartitionOutcome['results'][number] => {
      const result = jsonObject(entry, 'case result')
      const status = stringField(result, 'status')
      if (status !== 'pass' && status !== 'xfail' && status !== 'fail')
        throw new Error('Invalid case status')
      return { id: stringField(result, 'id'), status }
    },
  )
  if (typeof object.complete !== 'boolean')
    throw new Error('Missing receipt completion')
  return {
    partition,
    results,
    expectedIds: strings(object.expectedIds, 'expectedIds'),
    complete: object.complete,
    startedAt: duration(object, 'startedAt'),
    startupMs: duration(object, 'startupMs'),
    casesMs: duration(object, 'casesMs'),
    cleanupMs: duration(object, 'cleanupMs'),
    finishedAt: duration(object, 'finishedAt'),
  }
}

/** Exact identities and expected statuses prevent duplicate rows, density skips, and XPASS from passing. */
export function verifyCoverage(
  selected: readonly CaseSpec[],
  receipt: PartitionOutcome,
): void {
  const expected = new Map(
    selected.map((entry) => [entry.id, entry.expectation.kind]),
  )
  const actual = new Map(
    receipt.results.map((entry) => [entry.id, entry.status]),
  )
  if (
    !receipt.complete ||
    actual.size !== receipt.results.length ||
    actual.size !== expected.size ||
    receipt.expectedIds.length !== expected.size ||
    new Set(receipt.expectedIds).size !== expected.size ||
    receipt.expectedIds.some((id) => !expected.has(id)) ||
    [...expected].some(([id, status]) => actual.get(id) !== status)
  )
    throw new Error(
      'Partition did not execute every selected case with its required pass/xfail result',
    )
}

async function readJson(path: string): Promise<JsonObject> {
  return jsonObject(JSON.parse(await readFile(path, 'utf8')), path)
}

async function launcherCleanup(runDir: string, code: number): Promise<void> {
  const result = await readJson(join(runDir, 'result.json'))
  const cleanup = await readJson(join(runDir, 'cleanup.json'))
  if (
    result.state !== 'finished' ||
    result.code !== code ||
    result.cleaned !== true ||
    !Array.isArray(result.cleanupErrors) ||
    result.cleanupErrors.length !== 0 ||
    cleanup.retained === true
  )
    throw new Error('Launcher did not prove successful cleanup')
  for (const key of ['profile', 'home', 'temp']) {
    const path = stringField(cleanup, key)
    try {
      await access(path)
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT')
        continue
      throw error
    }
    throw new Error(`Launcher left its ${key} behind`)
  }
}

/**
 * This is the terminal case's only pass-producing boundary. The child can only submit
 * pending evidence; the parent checks actual launcher artifacts and removed private paths
 * before recording its named pass. Missing proof or any cleanup failure fails act-017 itself.
 */
export async function finalizeTerminalPartition(options: {
  out: string
  runDir?: string
  code: number
  launcherFailure?: string
}): Promise<void> {
  const spec = cases.find((entry) => entry.execution === 'terminal-select')
  if (!spec || spec.execution !== 'terminal-select')
    throw new Error('Missing terminal select catalog entry')
  let result: CaseResult = {
    id: spec.id,
    file: spec.file,
    name: spec.name,
    durationMs: 0,
    cleanupMs: 0,
    runIds: [],
    status: 'fail',
    failure: 'Terminal select has no cleanup proof',
  }
  let receiptInput: JsonObject = {
    startedAt: Date.now(),
    startupMs: 0,
    casesMs: 0,
    cleanupMs: 0,
  }
  let receiptFailure: string | undefined
  try {
    receiptInput = await readJson(join(options.out, 'receipt.json'))
  } catch (error) {
    receiptFailure = `Terminal suite receipt: ${error instanceof Error ? error.message : String(error)}`
  }
  try {
    if (receiptFailure) throw new Error(receiptFailure)
    if (options.launcherFailure) throw new Error(options.launcherFailure)
    if (!options.runDir)
      throw new Error('Terminal select launcher omitted cleanup artifacts')
    await launcherCleanup(options.runDir, options.code)
    await requireTerminalBrowserStopped(options.out, options.runDir)
    if (options.code !== 0)
      throw new Error(`Terminal select launcher failed (${options.code})`)
    if (
      receiptInput.partition !== 'terminal-select' ||
      receiptInput.complete !== false ||
      receiptInput.failure !== undefined ||
      !Array.isArray(receiptInput.results) ||
      receiptInput.results.length !== 0 ||
      JSON.stringify(receiptInput.expectedIds) !== JSON.stringify(['act-017'])
    )
      throw new Error(
        'Terminal child receipt must contain only pending coverage',
      )
    for (const key of [
      'startedAt',
      'startupMs',
      'casesMs',
      'cleanupMs',
      'finishedAt',
    ])
      duration(receiptInput, key)
    const pending = await readJson(join(options.out, 'pending-select.json'))
    if (
      pending.sdkStopped !== true ||
      pending.fixturesStopped !== true ||
      pending.failure !== null
    )
      throw new Error('Terminal select SDK/fixture cleanup was not proven')
    const sdk = await readJson(join(options.out, 'server.log.lifecycle.json'))
    if (
      sdk.cleaned !== true ||
      sdk.forced !== false ||
      sdk.exitCode !== 0 ||
      sdk.cleanupError !== undefined
    )
      throw new Error(
        'Terminal select SDK lifecycle receipt did not prove clean shutdown',
      )
    requireStoppedPid(sdk.pid, 'SDK')
    for (const key of ['root', 'socketPath']) {
      try {
        await access(stringField(sdk, key))
      } catch (error) {
        if (
          error instanceof Error &&
          'code' in error &&
          error.code === 'ENOENT'
        )
          continue
        throw error
      }
      throw new Error(`Terminal select SDK left its ${key} behind`)
    }
    const observation = readPendingSelect(pending.observation, spec)
    const passed: CaseResult = {
      id: spec.id,
      file: spec.file,
      name: spec.name,
      durationMs: observation.durationMs,
      cleanupMs: observation.cleanupMs,
      runIds: observation.runIds,
      status: 'pass',
    }
    await mkdir(join(options.out, 'cases'), { recursive: true })
    await writeFile(
      join(options.out, 'cases', 'act-017.json'),
      `${JSON.stringify(
        {
          id: spec.id,
          taskId: observation.taskId,
          requestId: observation.requestId,
          steps: [
            {
              scripts: spec.steps.flatMap((step) =>
                step.kind === 'script' ? [step.script] : step.scripts,
              ),
              replies: [observation.raw],
            },
          ],
          cleanup: observation.taskCleanup,
          launcherCleanupVerified: true,
          result: passed,
        },
        null,
        2,
      )}\n`,
      { mode: 0o600 },
    )
    result = passed
  } catch (error) {
    // Retain an already named child failure when the observation failed before pending state.
    const childFailures = Array.isArray(receiptInput.results)
      ? receiptInput.results
      : []
    const childFailure = childFailures.find(
      (entry) => jsonObject(entry, 'Terminal result').id === spec.id,
    )
    const detail = childFailure
      ? jsonObject(childFailure, 'Terminal result').failure
      : undefined
    result = {
      ...result,
      status: 'fail',
      failure: [
        typeof detail === 'string' ? detail : undefined,
        error instanceof Error ? error.message : String(error),
      ]
        .filter(Boolean)
        .join('; '),
    }
  }
  const receipt: PartitionReceipt = {
    partition: 'terminal-select',
    startedAt: safeDuration(receiptInput, 'startedAt'),
    startupMs: safeDuration(receiptInput, 'startupMs'),
    casesMs: safeDuration(receiptInput, 'casesMs'),
    caseCleanupMs: result.cleanupMs,
    cleanupMs: safeDuration(receiptInput, 'cleanupMs'),
    finishedAt: safeDuration(receiptInput, 'finishedAt'),
    finalizedAt: Date.now(),
    expectedIds: [spec.id],
    results: [result],
    complete: result.status === 'pass',
  }
  await writeFile(
    join(options.out, 'receipt.json'),
    `${JSON.stringify(receipt, null, 2)}\n`,
    { mode: 0o600 },
  )
  await writeFile(join(options.out, 'junit.xml'), junit(receipt), {
    mode: 0o600,
  })
}

/** Launcher receipts must describe this browser, not merely some removed directory. */
async function requireTerminalBrowserStopped(
  out: string,
  runDir: string,
): Promise<void> {
  const [browser, owner, cleanup, result] = await Promise.all([
    readJson(join(out, 'browser.json')),
    readJson(join(runDir, 'owner.json')),
    readJson(join(runDir, 'cleanup.json')),
    readJson(join(runDir, 'result.json')),
  ])
  for (const key of ['profile', 'home']) {
    const path = stringField(browser, key)
    if (owner[key] !== path || cleanup[key] !== path)
      throw new Error(`Terminal launcher ${key} belongs to another browser`)
  }
  if (cleanup.temp !== owner.temp || cleanup.pid !== owner.pid)
    throw new Error(
      'Terminal browser cleanup owner differs from its launch record',
    )
  requireStoppedPid(owner.pid, 'browser', true)
  requireStoppedPid(
    jsonObject(result.command, 'Launcher command').pid,
    'command',
    true,
  )
}

function requireStoppedPid(value: unknown, label: string, group = false): void {
  if (
    typeof value !== 'number' ||
    !Number.isSafeInteger(value) ||
    value <= 1 ||
    value > 2_147_483_647
  )
    throw new Error(`Terminal cleanup omitted a valid ${label} PID`)
  try {
    process.kill(group ? -value : value, 0)
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ESRCH')
      return
    throw error
  }
  throw new Error(
    `Terminal ${label} ${group ? 'process group' : 'process'} survived cleanup`,
  )
}

function safeDuration(object: JsonObject, key: string): number {
  const value = object[key]
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? value
    : 0
}

async function runPartition(
  partition: Partition,
  selected: readonly CaseSpec[],
  options: RunOptions,
  binary: string,
  browser: string,
) {
  const out = join(options.out, partition)
  await mkdir(out, { mode: 0o700 })
  const recordPath = join(out, 'browser.json')
  const command = [
    'run',
    'test:browser',
    '--',
    process.execPath,
    SUITE,
    '--partition',
    partition,
    '--out',
    out,
  ]
  if (options.caseId) command.push('--case', options.caseId)
  const startedAt = Date.now()
  const child = spawn(process.execPath, command, {
    cwd: AGENT_ROOT,
    env: {
      ...process.env,
      NEO_E2E_BROWSER: ADAPTER,
      SDK_DESKTOP_FIXTURE_BINARY: binary,
      SDK_DESKTOP_FIXTURE_BROWSER: browser,
      SDK_DESKTOP_FIXTURE_SCALE: partition === 'density' ? '2' : '1',
      SDK_DESKTOP_FIXTURE_BROWSER_RECORD: recordPath,
      SDK_DESKTOP_FIXTURE_DOWNLOADS: join(out, 'downloads'),
    },
    stdio: ['ignore', 'inherit', 'pipe'],
  })
  const release = forwardSignals(child)
  let runDir: string | undefined
  let pending = ''
  child.stderr.setEncoding('utf8')
  child.stderr.on('data', (chunk: string) => {
    process.stderr.write(chunk)
    pending += chunk
    const lines = pending.split('\n')
    pending = lines.pop() ?? ''
    for (const line of lines) {
      const match = line.match(/^\[test-browser\] artifacts: (.+)$/)
      if (match?.[1]) runDir = match[1].trim()
    }
  })
  let code: number
  try {
    code = await exited(child)
  } finally {
    release()
  }
  const finishedAt = Date.now()
  await writeFile(
    join(out, 'launch.json'),
    `${JSON.stringify(
      {
        partition,
        startedAt,
        finishedAt,
        durationMs: finishedAt - startedAt,
        launcherArtifacts: runDir,
        code,
      },
      null,
      2,
    )}\n`,
    { mode: 0o600 },
  )
  const validRunDir =
    runDir !== undefined &&
    dirname(runDir) === join(AGENT_ROOT, '.llm/test-browser') &&
    basename(runDir).startsWith('run-')
  if (partition === 'terminal-select') {
    await finalizeTerminalPartition({
      out,
      code,
      ...(validRunDir
        ? { runDir }
        : {
            launcherFailure:
              'Launcher did not publish its private artifact directory',
          }),
    })
  }
  if (
    !runDir ||
    dirname(runDir) !== join(AGENT_ROOT, '.llm/test-browser') ||
    !basename(runDir).startsWith('run-')
  )
    throw new Error('Launcher did not publish its private artifact directory')
  // A successful test command is insufficient when a browser group or private profile survived.
  if (partition !== 'terminal-select') await launcherCleanup(runDir, code)
  if (code !== 0)
    throw new Error(`${partition} launcher failed (${code}); see ${runDir}`)
  const receipt = partitionOutcome(await readJson(join(out, 'receipt.json')))
  if (receipt.partition !== partition)
    throw new Error('Partition receipt identity mismatch')
  verifyCoverage(selected, receipt)
  const launcherCleanupMs = Math.max(0, finishedAt - receipt.finishedAt)
  const durationMs = finishedAt - startedAt
  // This is the direct spawn-to-suite-main boundary. Evidence writes and reporting
  // gaps cannot be misattributed to startup by subtracting phase totals from wall time.
  const launcherStartupMs = Math.max(0, receipt.startedAt - startedAt)
  return {
    partition,
    startedAt,
    finishedAt,
    durationMs,
    launcherStartupMs,
    startupMs: receipt.startupMs,
    casesMs: receipt.casesMs,
    suiteCleanupMs: receipt.cleanupMs,
    launcherCleanupMs,
    launcherArtifacts: runDir,
    cleaned: true,
    code,
    results: receipt.results,
  }
}

function xmlEscape(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('"', '&quot;')
}

async function writeJunit(
  out: string,
  partitions: Partition[],
  failure?: string,
): Promise<void> {
  const suites: string[] = []
  for (const partition of partitions) {
    try {
      const xml = await readFile(join(out, partition, 'junit.xml'), 'utf8')
      const entries = xml.match(/<testsuite\b[^>]*>[\s\S]*?<\/testsuite>/g)
      if (!entries?.length) throw new Error('Partition JUnit contains no suite')
      suites.push(...entries)
    } catch (error) {
      if (!failure) throw error
    }
  }
  if (failure)
    suites.push(
      `<testsuite name="sdk-desktop-fixture-harness" tests="1" failures="1"><testcase name="source build and browser cleanup"><failure message="${xmlEscape(failure)}"/></testcase></testsuite>`,
    )
  const xml = `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites name="SDK desktop fixtures">\n${suites.join('\n')}\n</testsuites>\n`
  await writeFile(join(out, 'junit.xml'), xml)
  if (process.env.BROWSEROS_JUNIT_PATH) {
    const destination = resolve(AGENT_ROOT, process.env.BROWSEROS_JUNIT_PATH)
    await mkdir(dirname(destination), { recursive: true })
    await writeFile(destination, xml)
  }
}

async function main(): Promise<void> {
  if (process.argv.includes('--help')) {
    console.error(
      'Usage: bun crates/neo-sdk/neo-server/tests/desktop-fixtures/run.ts [--check] [--case act-001] [--out <directory>]',
    )
    return
  }
  const options = parseOptions(process.argv.slice(2))
  if (options.check) {
    await validateInventory()
    if (options.caseId && !cases.some((entry) => entry.id === options.caseId))
      throw new Error(`Unknown case ${options.caseId}`)
    console.error(`Validated ${cases.length} SDK desktop fixture cases`)
    return
  }
  await mkdir(options.out, { recursive: true, mode: 0o700 })
  const startedAt = Date.now()
  await writeFile(join(options.out, 'run.json'), '{}\n', {
    flag: 'wx',
    mode: 0o600,
  })
  let sourceBuild: Awaited<ReturnType<typeof buildSource>> | undefined
  const launches: Awaited<ReturnType<typeof runPartition>>[] = []
  const partitions: Partition[] = []
  let failure: string | undefined
  try {
    sourceBuild = await buildSource(options.out)
    await validateInventory()
    const selected = options.caseId
      ? cases.filter((entry) => entry.id === options.caseId)
      : cases
    if (selected.length === 0) throw new Error(`Unknown case ${options.caseId}`)
    const browser = await resolveBrowser(process.env)
    // The unknown-select contract always gets the final fresh launcher. No later case can
    // reuse it, regardless of whether observing the contract or its cleanup succeeds.
    for (const partition of [
      'ordinary',
      'density',
      'terminal-select',
    ] satisfies Partition[]) {
      const entries = partitionCases(selected, partition)
      if (entries.length === 0) continue
      partitions.push(partition)
      launches.push(
        await runPartition(
          partition,
          entries,
          options,
          sourceBuild.binary,
          browser,
        ),
      )
    }
  } catch (error) {
    failure = error instanceof Error ? error.message : String(error)
    throw error
  } finally {
    await writeFile(
      join(options.out, 'run.json'),
      `${JSON.stringify(
        {
          startedAt,
          finishedAt: Date.now(),
          sourceBuild,
          launches,
          selectedCase: options.caseId,
          complete: !failure,
          failure,
        },
        null,
        2,
      )}\n`,
      { mode: 0o600 },
    )
    await writeJunit(options.out, partitions, failure)
  }
}

if (import.meta.main) {
  try {
    await main()
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  }
}
