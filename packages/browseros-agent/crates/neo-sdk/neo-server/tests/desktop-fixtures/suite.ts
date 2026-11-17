import { mkdir } from 'node:fs/promises'
import { isAbsolute, join, resolve } from 'node:path'
import {
  type FixturePair,
  startFixturePair,
} from '../../../../../contracts/claw-mcp/fixtures/server'
import { Harness } from './harness'
import { createHostClient } from './host-client'
import { cases, loadBody, loadCommon, validateInventory } from './inventory'
import { jsonObject, stringField } from './json'
import { createRunClient } from './mcp'
import type { BrowserRecord, CaseResult, ServerHandle } from './model'
import {
  coverageError,
  junit,
  type Partition,
  type PartitionReceipt,
} from './receipt'
import { borrowedTargets } from './root-cdp'
import { startServer } from './server'
import {
  observeMissingOptionSelect,
  type PendingSelect,
} from './terminal-select'

interface Options {
  partition: Partition
  out: string
  caseId?: string
}

function options(args: readonly string[]): Options {
  let partition: Partition | undefined
  let out: string | undefined
  let caseId: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const flag = args[index]
    const value = args[index + 1]
    if (value === undefined) throw new Error(`Missing value for ${flag}`)
    if (
      flag === '--partition' &&
      (value === 'ordinary' ||
        value === 'density' ||
        value === 'terminal-select')
    )
      partition = value
    else if (flag === '--out') out = resolve(value)
    else if (flag === '--case') caseId = value
    else throw new Error(`Unknown suite argument: ${flag}`)
    index += 1
  }
  if (!partition || !out)
    throw new Error('suite.ts requires --partition and --out from run.ts')
  return { partition, out, caseId }
}

function environment(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`run.ts did not supply ${name}`)
  return value
}

async function browserRecord(): Promise<BrowserRecord> {
  const input = jsonObject(
    await Bun.file(environment('SDK_DESKTOP_FIXTURE_BROWSER_RECORD')).json(),
    'Browser adapter record',
  )
  const profile = stringField(input, 'profile')
  const home = stringField(input, 'home')
  const downloads = stringField(input, 'downloads')
  if (![profile, home, downloads].every(isAbsolute))
    throw new Error('Browser adapter paths must be absolute')
  if (downloads !== environment('SDK_DESKTOP_FIXTURE_DOWNLOADS'))
    throw new Error('Browser and server downloads paths differ')
  const scale = input.scale
  if (scale !== 1 && scale !== 2)
    throw new Error('Browser adapter omitted native scale')
  if (!Array.isArray(input.args))
    throw new Error('Browser adapter omitted launch arguments')
  const args = input.args.map((arg) => {
    if (typeof arg !== 'string')
      throw new Error('Browser launch argument must be text')
    return arg
  })
  if (!args.includes(`--user-data-dir=${profile}`))
    throw new Error('Browser adapter record lost its launcher profile marker')
  if (scale === 2 && !args.includes('--force-device-scale-factor=2'))
    throw new Error('Density browser was not launched at native scale 2')
  return { profile, home, downloads, args, scale }
}

/** The outer launcher owns the browser; this process owns only fixtures, SDK and case tasks. */
export async function main(args = process.argv.slice(2)): Promise<number> {
  const startedAt = Date.now()
  const started = performance.now()
  const opts = options(args)
  await mkdir(join(opts.out, 'cases'), { recursive: true })
  const expected = cases.filter((spec) => {
    return (
      spec.execution === opts.partition &&
      (!opts.caseId || spec.id === opts.caseId)
    )
  })
  if (expected.length === 0)
    throw new Error('Suite selection contains no cases')
  const expectedIds = expected.map((spec) => spec.id)
  const results: CaseResult[] = []
  let pendingSelect: PendingSelect | undefined
  let fixtures: FixturePair | undefined
  let server: ServerHandle | undefined
  let failure: string | undefined
  let startupMs = 0
  let interrupted: string | undefined
  const signals: NodeJS.Signals[] = ['SIGINT', 'SIGTERM', 'SIGHUP']
  const handlers = signals.map((signal) => {
    const handler = () => {
      interrupted = `Interrupted by ${signal}`
      // The launcher signals the entire command group. Stopping the sidecar immediately
      // unblocks pending MCP reads so case cleanup and private-state removal can finish
      // during the launcher's drain window. The awaited stop below retains any failure.
      void server?.stop().catch(() => {})
    }
    process.on(signal, handler)
    return { signal, handler }
  })
  const checkInterrupted = () => {
    if (interrupted) throw new Error(interrupted)
  }
  try {
    await validateInventory()
    checkInterrupted()
    const browser = await browserRecord()
    if (browser.scale !== (opts.partition === 'density' ? 2 : 1))
      throw new Error('Browser scale does not match suite partition')
    const cdpPort = Number(environment('BROWSEROS_CDP_PORT'))
    if (!Number.isInteger(cdpPort) || cdpPort < 1 || cdpPort > 65535)
      throw new Error('Invalid launcher CDP port')
    fixtures = await startFixturePair()
    server = await startServer({
      cdpPort,
      binary: environment('SDK_DESKTOP_FIXTURE_BINARY'),
      browser,
      logPath: join(opts.out, 'server.log'),
    })
    const host = createHostClient({
      socketPath: server.socketPath,
      token: server.token,
    })
    const common = await loadCommon()
    const harness = new Harness({
      run: createRunClient(server.baseUrl),
      host,
      targets: borrowedTargets(cdpPort),
      base: fixtures.primary.origin,
      base2: `http://localhost:${fixtures.secondary.port}`,
      scale: browser.scale,
      common,
      loadBody,
      async recordEvidence(evidence) {
        await Bun.write(
          join(opts.out, 'cases', `${evidence.id}.json`),
          `${JSON.stringify(evidence, null, 2)}\n`,
        )
      },
    })
    startupMs = performance.now() - started
    for (const spec of expected) {
      checkInterrupted()
      if (spec.execution === 'terminal-select') {
        if (opts.partition !== 'terminal-select' || expected.length !== 1)
          throw new Error(
            'Terminal select requires its own final disposable browser',
          )
        const step = spec.steps[0]
        if (step?.kind !== 'script')
          throw new Error('Terminal select requires one script')
        const check = await observeMissingOptionSelect({
          spec,
          baseUrl: server.baseUrl,
          host,
          common,
          base: fixtures.primary.origin,
          base2: `http://localhost:${fixtures.secondary.port}`,
          body: await loadBody(step.script),
        })
        if (check.kind === 'failed') results.push(check.result)
        else pendingSelect = check.observation
        // Even a failed observation seals the disposable browser. Only suite cleanup follows.
        break
      }
      const result = await harness.check(spec)
      results.push(result)
      console.log(
        `${result.status.toUpperCase()} ${result.file} ${result.id} ${result.name}`,
      )
      if (result.status === 'fail') console.error(result.failure)
      if (harness.poisoned) {
        failure = 'Case lifecycle is uncertain; browser state cannot be reused'
        break
      }
    }
  } catch (error) {
    failure = error instanceof Error ? error.message : String(error)
    if (!startupMs) startupMs = performance.now() - started
  }

  const cleanupStarted = performance.now()
  const cleanupErrors: string[] = []
  try {
    await server?.stop()
  } catch (error) {
    cleanupErrors.push(
      `SDK cleanup: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
  try {
    await fixtures?.stop()
  } catch (error) {
    cleanupErrors.push(
      `Fixture cleanup: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
  // A pending observation is deliberately absent from passing coverage and JUnit. Its
  // enclosing launcher is still alive; the parent alone can turn it into a named result.
  const coverage =
    pendingSelect && results.length === 0
      ? undefined
      : coverageError(expectedIds, results)
  const failures = [failure, interrupted, coverage, ...cleanupErrors].filter(
    (message): message is string => message !== undefined,
  )
  const caseCleanupMs = results.reduce(
    (total, result) => total + result.cleanupMs,
    pendingSelect?.cleanupMs ?? 0,
  )
  const receipt: PartitionReceipt = {
    partition: opts.partition,
    startedAt,
    startupMs,
    casesMs: results.reduce(
      (total, result) => total + result.durationMs,
      pendingSelect?.durationMs ?? 0,
    ),
    caseCleanupMs,
    cleanupMs: caseCleanupMs + performance.now() - cleanupStarted,
    finishedAt: Date.now(),
    expectedIds,
    results,
    complete:
      failures.length === 0 &&
      results.every((result) => result.status !== 'fail'),
    ...(failures.length ? { failure: failures.join('; ') } : {}),
  }
  if (pendingSelect) {
    await Bun.write(
      join(opts.out, 'pending-select.json'),
      `${JSON.stringify(
        {
          observation: pendingSelect,
          sdkStopped:
            server !== undefined &&
            cleanupErrors.every((error) => !error.startsWith('SDK cleanup:')),
          fixturesStopped:
            fixtures !== undefined &&
            cleanupErrors.every(
              (error) => !error.startsWith('Fixture cleanup:'),
            ),
          failure: failures.length ? failures.join('; ') : null,
        },
        null,
        2,
      )}\n`,
    )
    receipt.complete = false
    // No pass or terminal JUnit is emitted while browser/private-state destruction is pending.
  } else if (opts.partition !== 'terminal-select') {
    await Bun.write(join(opts.out, 'junit.xml'), junit(receipt))
  }
  await Bun.write(
    join(opts.out, 'receipt.json'),
    `${JSON.stringify(receipt, null, 2)}\n`,
  )
  for (const { signal, handler } of handlers) process.off(signal, handler)
  if (receipt.failure) console.error(receipt.failure)
  console.log(
    `${opts.partition}: ${results.filter((result) => result.status === 'pass').length} pass, ${results.filter((result) => result.status === 'xfail').length} xfail, ${results.filter((result) => result.status === 'fail').length} fail`,
  )
  return pendingSelect && failures.length === 0 ? 0 : receipt.complete ? 0 : 1
}

if (import.meta.main) {
  main()
    .then((code) => {
      process.exitCode = code
    })
    .catch((error: unknown) => {
      console.error(error instanceof Error ? error.message : String(error))
      process.exitCode = 1
    })
}
