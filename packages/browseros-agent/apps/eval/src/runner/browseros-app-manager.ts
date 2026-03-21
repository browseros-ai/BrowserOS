/**
 * BrowserOS App Manager
 *
 * Manages BrowserOS lifecycle for eval workers.
 * Mirrors scripts/dev/start.ts --manual mode with per-worker isolation:
 *
 *   1. Kill ports
 *   2. Build extensions (once, shared across workers)
 *   3. Launch Chrome directly with per-worker user-data-dir and ports
 *   4. Wait for CDP
 *   5. Start server with port env vars
 *   6. Wait for server health
 *
 * Each worker gets isolated ports: base + workerIndex offset.
 */

import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { type Subprocess, spawn, spawnSync } from 'bun'
import type { EvalPorts } from '../utils/dev-config'
import { sleep } from '../utils/sleep'

const MAX_RESTART_ATTEMPTS = 3
const CDP_WAIT_TIMEOUT_MS = 30_000
const SERVER_HEALTH_TIMEOUT_MS = 30_000

const MONOREPO_ROOT = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../../..',
)

const BROWSEROS_BINARY =
  process.env.BROWSEROS_BINARY ||
  '/Applications/BrowserOS.app/Contents/MacOS/BrowserOS'

const CONTROLLER_EXT_DIR = join(MONOREPO_ROOT, 'apps/controller-ext/dist')
const CAPTCHA_EXT_DIR = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../../extensions/nopecha',
)

export class BrowserOSAppManager {
  private ports: EvalPorts
  private chromeProc: Subprocess | null = null
  private serverProc: Subprocess | null = null
  private tempDir: string | null = null
  private readonly workerIndex: number
  private readonly loadExtensions: boolean
  private readonly headless: boolean

  constructor(
    workerIndex: number = 0,
    basePorts?: EvalPorts,
    loadExtensions: boolean = false,
    headless: boolean = false,
  ) {
    this.workerIndex = workerIndex
    this.loadExtensions = loadExtensions
    this.headless = headless
    const base = basePorts ?? { cdp: 9010, server: 9110, extension: 9310 }
    this.ports = {
      cdp: base.cdp + workerIndex,
      server: base.server + workerIndex,
      extension: base.extension + workerIndex,
    }
  }

  getServerUrl(): string {
    return `http://127.0.0.1:${this.ports.server}`
  }

  getPorts(): EvalPorts {
    return this.ports
  }

  /**
   * Build extensions (call once before starting workers).
   * Builds controller-ext — same as start.ts buildExtension('controller-ext', 'build:ext')
   */
  static buildExtensions(): void {
    console.log(`[BROWSEROS] Building controller extension...`)
    const result = spawnSync({
      cmd: ['bun', 'run', 'build:ext'],
      cwd: MONOREPO_ROOT,
      stdout: 'inherit',
      stderr: 'inherit',
    })
    if (result.exitCode !== 0) {
      throw new Error('Failed to build controller extension')
    }
    console.log(`[BROWSEROS] Controller extension built`)
  }

  /**
   * Restart: kill existing, then start fresh
   */
  async restart(): Promise<void> {
    for (let attempt = 1; attempt <= MAX_RESTART_ATTEMPTS; attempt++) {
      console.log(
        `  [W${this.workerIndex}] Restart attempt ${attempt}/${MAX_RESTART_ATTEMPTS}...`,
      )

      await this.killApp()
      await sleep(2000)

      try {
        await this.startAll()
        console.log(`  [W${this.workerIndex}] Ready`)
        return
      } catch (error) {
        console.warn(
          `  [W${this.workerIndex}] Start failed (attempt ${attempt}): ${error instanceof Error ? error.message : String(error)}`,
        )
      }
    }

    throw new Error(
      `Failed to start BrowserOS after ${MAX_RESTART_ATTEMPTS} attempts`,
    )
  }

  /**
   * Launch Chrome + Server — mirrors start.ts --manual mode.
   *
   * Chrome flags match startManualBrowser() in scripts/dev/start.ts:
   *   --no-first-run, --no-default-browser-check, --use-mock-keychain
   *   --disable-browseros-server  (we run our own server)
   *   --disable-browseros-extensions  (we load them explicitly if needed)
   *   --remote-debugging-port, --browseros-mcp-port, --browseros-extension-port
   *   --user-data-dir (unique per worker)
   *   --load-extension (optional, controller-ext)
   */
  private async startAll(): Promise<void> {
    const { cdp, server, extension } = this.ports

    // Unique temp dir per worker per restart
    this.tempDir = mkdtempSync('/tmp/browseros-eval-')

    console.log(
      `  [W${this.workerIndex}] Ports: CDP=${cdp} Server=${server} Extension=${extension}${this.headless ? ' (headless)' : ''}`,
    )
    console.log(`  [W${this.workerIndex}] Profile: ${this.tempDir}`)

    // --- Chrome Launch (matches start.ts startManualBrowser) ---
    const chromeArgs = [
      '--no-first-run',
      '--no-default-browser-check',
      '--use-mock-keychain',
      '--disable-browseros-server',
      '--disable-browseros-extensions',
      '--incognito',
      ...(this.headless ? ['--headless=new'] : []),
      '--window-size=1440,900',
      `--remote-debugging-port=${cdp}`,
      `--browseros-mcp-port=${server}`,
      `--browseros-extension-port=${extension}`,
      `--user-data-dir=${this.tempDir}`,
    ]

    const extensions: string[] = []
    if (this.loadExtensions && existsSync(CONTROLLER_EXT_DIR)) {
      extensions.push(CONTROLLER_EXT_DIR)
    }
    if (existsSync(CAPTCHA_EXT_DIR)) {
      extensions.push(CAPTCHA_EXT_DIR)
    }
    if (extensions.length > 0) {
      chromeArgs.push(`--load-extension=${extensions.join(',')}`)
    }

    chromeArgs.push('about:blank')

    this.chromeProc = spawn({
      cmd: [BROWSEROS_BINARY, ...chromeArgs],
      stdout: 'ignore',
      stderr: 'ignore',
    })
    console.log(
      `  [W${this.workerIndex}] Chrome started (PID: ${this.chromeProc.pid})`,
    )

    // --- Wait for CDP ---
    if (!(await this.waitForCdp())) {
      throw new Error('CDP not available after timeout')
    }
    console.log(`  [W${this.workerIndex}] CDP ready`)

    // --- Server Launch (matches start.ts createEnv + startServer) ---
    const serverEnv = {
      ...process.env,
      NODE_ENV: 'development',
      BROWSEROS_CDP_PORT: String(cdp),
      BROWSEROS_SERVER_PORT: String(server),
      BROWSEROS_EXTENSION_PORT: String(extension),
      VITE_BROWSEROS_SERVER_PORT: String(server),
    }

    this.serverProc = spawn({
      cmd: ['bun', 'run', '--filter', '@browseros/server', 'start'],
      cwd: MONOREPO_ROOT,
      stdout: 'ignore',
      stderr: 'ignore',
      env: serverEnv,
    })
    console.log(
      `  [W${this.workerIndex}] Server started (PID: ${this.serverProc.pid})`,
    )

    // --- Wait for Server Health ---
    if (!(await this.waitForServerHealth())) {
      throw new Error('Server health check timed out')
    }
    console.log(`  [W${this.workerIndex}] Server healthy`)
  }

  private async waitForCdp(): Promise<boolean> {
    const startTime = Date.now()
    while (Date.now() - startTime < CDP_WAIT_TIMEOUT_MS) {
      try {
        const res = await fetch(
          `http://127.0.0.1:${this.ports.cdp}/json/version`,
          { signal: AbortSignal.timeout(1000) },
        )
        if (res.ok) return true
      } catch {
        // not ready
      }
      await sleep(500)
    }
    return false
  }

  private async waitForServerHealth(): Promise<boolean> {
    const startTime = Date.now()
    while (Date.now() - startTime < SERVER_HEALTH_TIMEOUT_MS) {
      try {
        const res = await fetch(
          `http://127.0.0.1:${this.ports.server}/health`,
          { signal: AbortSignal.timeout(1000) },
        )
        if (res.ok) return true
      } catch {
        // not ready
      }
      await sleep(500)
    }
    return false
  }

  /**
   * Kill Chrome + Server, clean up temp dir.
   * Mirrors start.ts cleanup but per-worker (port-based, not pgrep).
   */
  async killApp(): Promise<void> {
    // Kill server first (graceful → force)
    await this.killProcess(this.serverProc)
    this.serverProc = null

    // Kill Chrome (graceful → force)
    await this.killProcess(this.chromeProc)
    this.chromeProc = null

    await sleep(1000)

    // Force kill anything still on our ports
    if (this.isAppRunning()) {
      for (const port of [
        this.ports.cdp,
        this.ports.server,
        this.ports.extension,
      ]) {
        spawnSync({
          cmd: [
            'sh',
            '-c',
            `lsof -ti:${port} -sTCP:LISTEN | xargs kill -9 2>/dev/null || true`,
          ],
        })
      }
    }

    // Clean up temp dir
    if (this.tempDir) {
      try {
        rmSync(this.tempDir, { recursive: true, force: true })
      } catch {
        // ignore
      }
      this.tempDir = null
    }
  }

  private async killProcess(proc: Subprocess | null): Promise<void> {
    if (!proc) return
    try {
      proc.kill('SIGTERM')
      await Promise.race([proc.exited, sleep(2000)])
      try {
        proc.kill('SIGKILL')
      } catch {
        // already dead
      }
    } catch {
      // already dead
    }
  }

  /**
   * Check if anything is listening on our server port (port-specific, not pgrep)
   */
  isAppRunning(): boolean {
    const result = spawnSync({
      cmd: [
        'sh',
        '-c',
        `lsof -ti:${this.ports.server} -sTCP:LISTEN 2>/dev/null`,
      ],
    })
    return (result.stdout?.toString().trim() ?? '').length > 0
  }
}
