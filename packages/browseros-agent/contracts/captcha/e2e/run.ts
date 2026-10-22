import { mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { parseArgs } from 'node:util'
import { waitUntil } from '../../claw-mcp/tests/helpers'
import { McpSession, textOf } from '../../claw-mcp/tests/mcp-client'
import {
  buildRustServer,
  RUST_BINARY,
  startRustServer,
} from '../../claw-mcp/tests/rust-server'
import type { CaptchaPage } from '../fixtures/catalog'
import { launchConfiguredBrowser } from './browser-launch'
import { configureKeys, type KeyMode } from './keys'
import { createResources } from './lifecycle'
import { exitCode, renderTable, runCaptchaE2E } from './runner'

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
      pages: { type: 'string', default: 'fixtures' },
      only: { type: 'string' },
      keys: { type: 'string', default: 'placeholder' },
      paid: { type: 'boolean' },
      'expect-tool-missing': { type: 'boolean' },
      'mcp-url': { type: 'string' },
      'fixture-bind': { type: 'string' },
      'fixture-host': { type: 'string' },
      'fixture-origin': { type: 'string' },
      headed: { type: 'boolean' },
      'browser-arg': { type: 'string', multiple: true },
      'solve-timeout': { type: 'string', default: '150000' },
      out: { type: 'string' },
    },
  })
  const solveTimeout = Number(values['solve-timeout'])
  if (!['fixtures', 'demo', 'all'].includes(values.pages))
    throw new Error('--pages must be fixtures, demo, or all')
  if (!['placeholder', 'real', 'none'].includes(values.keys))
    throw new Error('--keys must be placeholder, real, or none')
  if (
    !Number.isInteger(solveTimeout) ||
    solveTimeout < 1 ||
    solveTimeout > 2_147_483_647
  )
    throw new Error('--solve-timeout must be positive milliseconds')
  if (values.only?.split(',').some((id) => !id.trim()))
    throw new Error('--only needs catalog ids')
  for (const value of [values['mcp-url'], values['fixture-origin']])
    if (
      value !== undefined &&
      !['http:', 'https:'].includes(new URL(value).protocol)
    )
      throw new Error('URLs must use HTTP(S)')
  return {
    pages: values.pages,
    only: values.only?.split(',').map((id) => id.trim()),
    keys: values.keys as KeyMode,
    paid: values.paid,
    expectToolMissing: values['expect-tool-missing'],
    mcpUrl: values['mcp-url'],
    fixtureBind: values['fixture-bind'],
    fixtureHost: values['fixture-host'],
    fixtureOrigin: values['fixture-origin'],
    headed: values.headed ?? false,
    browserArgs: values['browser-arg'] ?? [],
    solveTimeout,
    out: values.out ?? join(tmpdir(), `captcha-e2e-${Date.now()}`),
  }
}

export function selectPages(
  pages: readonly CaptchaPage[],
  options: ReturnType<typeof parseOptions>,
) {
  const selected = pages.filter(
    (p) =>
      (options.pages === 'all' ||
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

async function main(): Promise<number> {
  const options = parseOptions(Bun.argv.slice(2))
  if (!options.mcpUrl && !process.env.BROWSEROS_BINARY)
    throw new Error('BROWSEROS_BINARY is required unless --mcp-url is set')
  const catalog = await import('../fixtures/catalog')
  const pages = selectPages(
    [...catalog.FIXTURE_PAGES, ...catalog.DEMO_PAGES],
    options,
  )
  const resources = createResources()
  const interrupt = () => {
    void resources.close().finally(() => process.exit(130))
  }
  process.once('SIGINT', interrupt)
  process.once('SIGTERM', interrupt)
  try {
    const keys = await resources.own(
      () => configureKeys(options.keys),
      (keys) => keys.cleanup(),
    )
    let fixtureOrigin = options.fixtureOrigin
    if (!fixtureOrigin) {
      const { startCaptchaFixtureServer } = await import('../fixtures/server')
      const fixture = await resources.own(
        () =>
          startCaptchaFixtureServer({
            hostname: options.fixtureBind,
            publicHost: options.fixtureHost,
          }),
        (fixture) => fixture.stop(),
      )
      fixtureOrigin = fixture.origin
    }
    let mcpUrl = options.mcpUrl
    if (!mcpUrl) {
      if (!process.env.CLAW_SERVER_BINARY) buildRustServer()
      const browser = await resources.own(
        () => launchConfiguredBrowser(options.headed, options.browserArgs),
        (browser) => browser.kill(),
      )
      const server = await resources.own(
        () => startRustServer(browser.cdpPort, { env: keys.env }),
        (server) => server.stop(),
      )
      mcpUrl = server.baseUrl
      const session = await McpSession.connect(mcpUrl, 'captcha-e2e-attach')
      try {
        await waitUntil(
          async () => {
            const result = await session.callTool('tabs', { action: 'list' })
            return !(
              result.isError &&
              textOf(result).includes('browser session not connected')
            )
          },
          'browser attach',
          { timeoutMs: 30_000, intervalMs: 500 },
        )
      } finally {
        await session.close().catch(() => {})
      }
    }
    const records = await runCaptchaE2E({
      ...options,
      pages,
      mcpUrl,
      fixtureOrigin,
      tokenLengthExpression: catalog.tokenLengthExpression,
    })
    const sha =
      Bun.spawnSync(['git', 'rev-parse', 'HEAD']).stdout.toString().trim() ||
      'unknown'
    const header = `# CAPTCHA E2E\n\nTime: ${new Date().toISOString()}\n\nGit: ${sha}\n\nServer: ${options.mcpUrl ?? RUST_BINARY}\n\nKeys: ${options.keys}; browser: ${options.headed ? 'headed' : 'headless'}\n\n`
    const table = renderTable(records)
    const summary = ['PASS', 'FAIL', 'UNVERIFIED']
      .map(
        (outcome) =>
          `${outcome}=${records.filter((r) => r.outcome === outcome).length}`,
      )
      .join(' ')
    await mkdir(options.out, { recursive: true })
    await writeFile(
      join(options.out, 'results.json'),
      `${JSON.stringify(records, null, 2)}\n`,
    )
    await writeFile(
      join(options.out, 'results.md'),
      `${header}${table}\n\n${summary}\n`,
    )
    console.log(`${header}${table}\n\n${summary}\nResults: ${options.out}`)
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
