import {
  afterAll,
  beforeAll,
  describe,
  expect,
  it,
  setDefaultTimeout,
} from 'bun:test'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { cleanupBrowserOS, ensureBrowserOS } from '../__helpers__/index'

setDefaultTimeout(120000)

const serverRoot = resolve(import.meta.dir, '..', '..')
const cliRoot = resolve(serverRoot, '..', 'cli')

let cliBinary = ''
let serverUrl = ''
let tmpDir = ''

interface CliResult {
  stdout: string
  stderr: string
  status: number
}

/** Builds the Go CLI once so tests exercise the shipped command binary shape. */
function buildCli(): void {
  tmpDir = mkdtempSync(join(tmpdir(), 'browseros-cli-runtime-'))
  cliBinary = join(tmpDir, 'browseros-cli')

  const result = spawnSync('go', ['build', '-o', cliBinary, '.'], {
    cwd: cliRoot,
    encoding: 'utf8',
  })
  if (result.error) {
    throw result.error
  }
  if (result.status !== 0) {
    throw new Error(
      `failed to build browseros-cli\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
    )
  }
}

/** Runs the built CLI against the BrowserOS test server without reading user config. */
function runCli(args: string[]): CliResult {
  const result = spawnSync(cliBinary, ['--server', serverUrl, ...args], {
    cwd: cliRoot,
    encoding: 'utf8',
    env: {
      ...process.env,
      BROWSEROS_SKIP_UPDATE_CHECK: '1',
    },
  })
  if (result.error) {
    throw result.error
  }
  return {
    stdout: result.stdout,
    stderr: result.stderr,
    status: result.status ?? 1,
  }
}

/** Parses JSON CLI output and includes stderr/stdout in assertion failures. */
function runJson(args: string[]): Record<string, unknown> {
  const result = runCli(['--json', ...args])
  if (result.status !== 0) {
    throw new Error(
      `browseros-cli ${args.join(' ')} exited ${result.status}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
    )
  }
  try {
    return JSON.parse(result.stdout.trim()) as Record<string, unknown>
  } catch (error) {
    throw new Error(
      `browseros-cli ${args.join(' ')} returned invalid JSON: ${error instanceof Error ? error.message : String(error)}\nstdout:\n${result.stdout}`,
    )
  }
}

describe('browseros-cli runtime commands', () => {
  beforeAll(async () => {
    buildCli()
    const config = await ensureBrowserOS()
    serverUrl = `http://127.0.0.1:${config.serverPort}`
  })

  afterAll(async () => {
    if (!process.env.KEEP_BROWSER) {
      await cleanupBrowserOS()
    }
    if (tmpDir) {
      rmSync(tmpDir, { recursive: true, force: true })
    }
  })

  it('checks server health', () => {
    const data = runJson(['health'])

    expect(data.status).toBe('ok')
    expect(data.cdpConnected).toBe(true)
  })

  it('lists pages through the tabs tool output', () => {
    const data = runJson(['pages'])

    expect(Array.isArray(data.pages)).toBe(true)
    expect(data.count).toBe((data.pages as unknown[]).length)
    expect((data.pages as unknown[]).length).toBeGreaterThan(0)
  })

  it('shows the active page through the tabs tool output', () => {
    const data = runJson(['active'])

    expect(typeof data.page).toBe('number')
    expect(typeof data.tabId).toBe('number')
  })

  it('snap captures an explicit page', () => {
    const active = runJson(['active'])
    const page = active.page

    expect(typeof page).toBe('number')
    const data = runJson(['-p', String(page), 'snap'])
    expect(typeof data.page).toBe('number')
    expect(typeof data.snapshot).toBe('string')
    expect((data.snapshot as string).length).toBeGreaterThan(10)
  })

  it('snap requires an explicit page flag', () => {
    const result = runCli(['--json', 'snap'])

    expect(result.status).toBe(2)
    expect(result.stderr).toContain('-p/--page')
  })
})
