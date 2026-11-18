import { mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { parseArgs } from 'node:util'
import { type CaptchaPage, SPOTTING_PAGES } from '../fixtures/catalog'
import { launchConfiguredBrowser } from './browser-launch'
import { startDevWatch } from './dev-watch'
import { configureKeys, type KeyMode } from './keys'
import { createResources } from './lifecycle'
import { checkPreflight, renderPreflight, type Server } from './preflight'
import {
  exitCode,
  progressLine,
  renderSummary,
  renderTable,
  runCaptchaE2E,
} from './runner'
import { prepareSdkServer, startSdkServer } from './sdk-server'

/** The per-process paid-task budget a launched server gets with real keys, unless set. */
export const DEFAULT_REAL_PAID_TASK_LIMIT = 12
const milliseconds = (value: string, flag: string, max = 2_147_483_647) => {
  const number = Number(value)
  if (!Number.isInteger(number) || number < 1 || number > max)
    throw new Error(`${flag} must be 1 to ${max} milliseconds`)
  return number
}
/** The longest `run` the SDK accepts. */
const RUN_TIMEOUT_MAX = 900_000
/** neo-captcha's own limit for one solve. */
const SOLVER_LIMIT = 120_000

export function parseOptions(argv: string[]) {
  const args = argv[0] === '--' ? argv.slice(1) : argv
  const normalized: string[] = []
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--browser-arg' && args[i + 1] !== undefined)
      normalized.push(`--browser-arg=${args[++i]}`)
    else normalized.push(args[i] as string)
  }
  const { values } = parseArgs({
    args: normalized,
    options: {
      server: { type: 'string', default: 'sdk' },
      pages: { type: 'string', default: 'fixtures' },
      only: { type: 'string' },
      keys: { type: 'string', default: 'placeholder' },
      paid: { type: 'boolean' },
      'paid-task-limit': { type: 'string' },
      'expect-tool-missing': { type: 'boolean' },
      'mcp-url': { type: 'string' },
      'dev-watch': { type: 'boolean' },
      'startup-timeout': { type: 'string', default: '600000' },
      'fixture-bind': { type: 'string' },
      'fixture-host': { type: 'string' },
      'fixture-frame-host': { type: 'string' },
      'fixture-origin': { type: 'string' },
      headed: { type: 'boolean' },
      headless: { type: 'boolean' },
      'browser-arg': { type: 'string', multiple: true },
      'solve-timeout': { type: 'string', default: '150000' },
      out: { type: 'string' },
    },
  })
  if (!['sdk', 'claw-server'].includes(values.server))
    throw new Error('--server must be sdk or claw-server')
  if (values.server === 'claw-server' && !values['mcp-url'])
    throw new Error(
      '--server claw-server needs --mcp-url: the testbed launches only the Neo SDK',
    )
  const startupTimeout = milliseconds(
    values['startup-timeout'],
    '--startup-timeout',
  )
  if (
    values['dev-watch'] &&
    (values['mcp-url'] || values.headless || values['browser-arg']?.length)
  )
    throw new Error(
      '--dev-watch uses its own headed browser; cannot combine with --mcp-url, --headless or --browser-arg',
    )
  if (!['fixtures', 'demo', 'all', 'spotting'].includes(values.pages))
    throw new Error('--pages must be fixtures, demo, all, or spotting')
  if (!['placeholder', 'real', 'none'].includes(values.keys))
    throw new Error('--keys must be placeholder, real, or none')
  const solveTimeout = milliseconds(
    values['solve-timeout'],
    '--solve-timeout',
    RUN_TIMEOUT_MAX,
  )
  if (values.only?.split(',').some((id) => !id.trim()))
    throw new Error('--only needs catalog ids')
  let paidTaskLimit: number | undefined
  if (values['paid-task-limit'] !== undefined) {
    paidTaskLimit = Number(values['paid-task-limit'])
    if (!Number.isInteger(paidTaskLimit) || paidTaskLimit < 0)
      throw new Error('--paid-task-limit must be a whole number')
    if (values['mcp-url'])
      throw new Error(
        '--paid-task-limit applies to servers the runner starts; an --mcp-url endpoint sets its own',
      )
  }
  for (const value of [values['mcp-url'], values['fixture-origin']])
    if (
      value !== undefined &&
      !['http:', 'https:'].includes(new URL(value).protocol)
    )
      throw new Error('URLs must use HTTP(S)')
  return {
    server: values.server as Server,
    pages: values.pages,
    only: values.only?.split(',').map((id) => id.trim()),
    keys: values.keys as KeyMode,
    paid: values.paid,
    paidTaskLimit:
      paidTaskLimit ??
      (values.keys === 'real' ? DEFAULT_REAL_PAID_TASK_LIMIT : undefined),
    expectToolMissing: values['expect-tool-missing'],
    mcpUrl: values['mcp-url'],
    devWatch: values['dev-watch'],
    startupTimeout,
    fixtureBind: values['fixture-bind'],
    fixtureHost: values['fixture-host'],
    fixtureFrameHost: values['fixture-frame-host'],
    fixtureOrigin: values['fixture-origin'],
    headed: !values.headless,
    browserArgs: values['browser-arg'] ?? [],
    solveTimeout,
    out: values.out ?? join(tmpdir(), `captcha-e2e-${Date.now()}`),
  }
}
export type Options = ReturnType<typeof parseOptions>

export function selectPages(pages: readonly CaptchaPage[], options: Options) {
  const selected = pages.filter(
    (p) =>
      (options.pages === 'spotting'
        ? SPOTTING_PAGES.some((spotting) => spotting.id === p.id)
        : options.pages === 'all' ||
          p.kind === (options.pages === 'fixtures' ? 'fixture' : 'demo')) &&
      (!options.only || options.only.includes(p.id)),
  )
  if (
    !selected.length ||
    options.only?.some((id) => !selected.some((p) => p.id === id))
  )
    throw new Error('no matching pages or unknown --only id')
  return selected
}

/** The environment of a server the runner starts: its keys, and its paid-task budget. */
export function serverEnv(
  keyEnv: Record<string, string | undefined>,
  paidTaskLimit: number | undefined,
): Record<string, string | undefined> {
  return {
    ...keyEnv,
    BROWSEROS_CAPTCHA_PAID_TASK_LIMIT:
      paidTaskLimit === undefined ? undefined : String(paidTaskLimit),
  }
}

function header(options: Options, mcpUrl: string) {
  const sha =
    Bun.spawnSync(['git', 'rev-parse', 'HEAD']).stdout.toString().trim() ||
    'unknown'
  const launched = options.mcpUrl
    ? 'endpoint-owned'
    : options.devWatch
      ? 'dev-watch (BROWSERCLAW_USE_SDK=1)'
      : 'local browseros-claw-sdk'
  return `# CAPTCHA E2E\n\nTime: ${new Date().toISOString()}\n\nGit: ${sha}\n\nServer: ${options.server} at ${mcpUrl} (${launched})\n\nKeys: ${options.keys}${options.paid ? ', paid' : ''}${options.paidTaskLimit !== undefined && !options.mcpUrl ? `; paid-task limit ${options.paidTaskLimit}` : ''}; browser: ${options.headed ? 'headed' : 'headless'}\n\n`
}

async function main(): Promise<number> {
  const options = parseOptions(Bun.argv.slice(2))
  const catalog = await import('../fixtures/catalog')
  const pages = selectPages(
    [
      ...catalog.FIXTURE_PAGES,
      ...catalog.DEMO_PAGES,
      ...catalog.SPOTTING_PAGES,
    ],
    options,
  )
  if (options.solveTimeout < SOLVER_LIMIT)
    console.warn(
      `--solve-timeout ${options.solveTimeout} is under the solver's own ${SOLVER_LIMIT} ms: a solve may be cut short after it paid`,
    )
  if (!options.mcpUrl && !options.devWatch && !process.env.BROWSEROS_BINARY)
    throw new Error(
      'BROWSEROS_BINARY is required unless --mcp-url or --dev-watch is set',
    )
  const resources = createResources()
  const interrupt = () => {
    void resources.close().then(
      () => process.exit(130),
      (error) => {
        console.error(String(error))
        process.exit(2)
      },
    )
  }
  process.on('SIGINT', interrupt)
  process.on('SIGTERM', interrupt)
  try {
    await mkdir(options.out, { recursive: true })
    let mcpUrl = options.mcpUrl
    if (!mcpUrl) {
      const keys = await resources.own(
        () => configureKeys(options.keys),
        (keys) => keys.cleanup(),
      )
      const env = serverEnv(keys.env, options.paidTaskLimit)
      if (options.devWatch) {
        const watch = await resources.own(
          () =>
            startDevWatch({
              out: options.out,
              startupTimeout: options.startupTimeout,
              serverEnv: env,
            }),
          (watch) => watch.stop(),
        )
        mcpUrl = await watch.ready()
      } else {
        prepareSdkServer()
        const browser = await resources.own(
          () => launchConfiguredBrowser(options.headed, options.browserArgs),
          (browser) => browser.kill(),
        )
        const server = await resources.own(
          () =>
            startSdkServer(browser.cdpPort, {
              env,
              logPath: join(options.out, 'sdk-server.log'),
            }),
          (server) => server.stop(),
        )
        mcpUrl = `${server.baseUrl}/mcp`
      }
    }
    const preflight = await checkPreflight(mcpUrl, {
      server: options.server,
      expectToolMissing: options.expectToolMissing,
      refuseOwnerPorts: options.devWatch,
    })
    const top = `${header(options, mcpUrl)}${renderPreflight(preflight)}`
    await writeFile(
      join(options.out, 'preflight.json'),
      `${JSON.stringify(preflight, null, 2)}\n`,
    )
    if (!preflight.ok) {
      await writeFile(join(options.out, 'results.json'), '[]\n')
      await writeFile(join(options.out, 'results.md'), top)
      console.error(`${top}Results: ${options.out}`)
      return preflight.blocker ? 2 : 1
    }
    console.log(renderPreflight(preflight))
    let fixtureOrigin = options.fixtureOrigin
    if (!fixtureOrigin) {
      const { startCaptchaFixtureServer } = await import('../fixtures/server')
      const fixture = await resources.own(
        () =>
          startCaptchaFixtureServer({
            hostname: options.fixtureBind,
            publicHost: options.fixtureHost,
            publicFrameHost: options.fixtureFrameHost,
          }),
        (fixture) => fixture.stop(),
      )
      fixtureOrigin = fixture.origin
    }
    const records = await runCaptchaE2E({
      ...options,
      pages,
      mcpUrl,
      fixtureOrigin,
      tokenLengthExpression: catalog.tokenLengthExpression,
      onRecord: (record) => console.log(progressLine(record)),
    })
    const report = `${top}${renderTable(records)}\n\n${renderSummary(records)}\n`
    await writeFile(
      join(options.out, 'results.json'),
      `${JSON.stringify(records, null, 2)}\n`,
    )
    await writeFile(join(options.out, 'results.md'), report)
    console.log(`${report}Results: ${options.out}`)
    return exitCode(records)
  } finally {
    await resources.close()
    process.removeListener('SIGINT', interrupt)
    process.removeListener('SIGTERM', interrupt)
  }
}

if (import.meta.main) {
  try {
    process.exitCode = await main()
  } catch (error) {
    console.error(
      error instanceof Error ? error.message : 'CAPTCHA runner boot failed',
    )
    process.exitCode = 2
  }
}
