/**
 * Manages WebArena-Infinity app server lifecycle per task.
 *
 * Each worker gets a unique port: base_port + worker_index.
 * Server is started fresh before each task and killed after,
 * guaranteeing clean state.
 */

import { type ChildProcess, spawn } from 'node:child_process'
import { join } from 'node:path'

export class InfinityAppManager {
  private proc: ChildProcess | null = null
  private port: number
  private infinityDir: string

  constructor(
    private workerIndex: number,
    private basePort: number = 8000,
  ) {
    this.port = basePort + workerIndex
    this.infinityDir = process.env.WEBARENA_INFINITY_DIR || ''
  }

  async startApp(appName: string): Promise<string> {
    await this.stop()

    if (!this.infinityDir) {
      throw new Error('WEBARENA_INFINITY_DIR env var not set')
    }

    const serverScript = join(this.infinityDir, 'apps', appName, 'server.py')
    this.proc = spawn('python3', [serverScript, '--port', String(this.port)], {
      stdio: ['ignore', 'pipe', 'pipe'],
      cwd: join(this.infinityDir, 'apps', appName),
    })

    // Wait for server to be ready
    const url = `http://localhost:${this.port}`
    await this.waitForReady(url)
    return url
  }

  async stop(): Promise<void> {
    if (this.proc) {
      this.proc.kill('SIGTERM')
      await new Promise<void>((resolve) => {
        const timeout = setTimeout(() => {
          this.proc?.kill('SIGKILL')
          resolve()
        }, 3000)
        this.proc?.on('exit', () => {
          clearTimeout(timeout)
          resolve()
        })
      })
      this.proc = null
    }
  }

  getPort(): number {
    return this.port
  }

  getUrl(): string {
    return `http://localhost:${this.port}`
  }

  private async waitForReady(
    url: string,
    maxAttempts = 30,
    intervalMs = 500,
  ): Promise<void> {
    for (let i = 0; i < maxAttempts; i++) {
      try {
        const resp = await fetch(url, {
          signal: AbortSignal.timeout(2000),
        })
        if (resp.ok) return
      } catch {
        // Server not ready yet
      }
      await new Promise((r) => setTimeout(r, intervalMs))
    }
    throw new Error(
      `Infinity app server not ready after ${maxAttempts * intervalMs}ms on port ${this.port}`,
    )
  }
}
