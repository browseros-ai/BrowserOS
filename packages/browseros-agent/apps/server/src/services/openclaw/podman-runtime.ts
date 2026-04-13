/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Abstraction over the Podman CLI for container lifecycle management.
 * Handles Podman machine init/start on macOS/Windows (where a Linux VM is required).
 * On Linux, machine operations are no-ops since Podman runs natively.
 */

const isLinux = process.platform === 'linux'

export type LogFn = (msg: string) => void

export class PodmanRuntime {
  private podmanPath: string
  private machineReady = false

  constructor(config?: { podmanPath?: string }) {
    this.podmanPath = config?.podmanPath ?? 'podman'
  }

  getPodmanPath(): string {
    return this.podmanPath
  }

  async isPodmanAvailable(): Promise<boolean> {
    try {
      const proc = Bun.spawn([this.podmanPath, '--version'], {
        stdout: 'ignore',
        stderr: 'ignore',
      })
      return (await proc.exited) === 0
    } catch {
      return false
    }
  }

  async getMachineStatus(): Promise<{
    initialized: boolean
    running: boolean
  }> {
    if (isLinux) return { initialized: true, running: true }

    try {
      const proc = Bun.spawn(
        [this.podmanPath, 'machine', 'list', '--format', 'json'],
        { stdout: 'pipe', stderr: 'ignore' },
      )
      const output = await new Response(proc.stdout).text()
      await proc.exited

      const machines = JSON.parse(output) as Array<{
        Running?: boolean
        LastUp?: string
      }>

      if (!machines.length) return { initialized: false, running: false }

      const machine = machines[0]
      const running =
        machine.Running === true || machine.LastUp === 'Currently running'

      return { initialized: true, running }
    } catch {
      return { initialized: false, running: false }
    }
  }

  async initMachine(onLog?: LogFn): Promise<void> {
    if (isLinux) return

    const proc = Bun.spawn(
      [
        this.podmanPath,
        'machine',
        'init',
        '--cpus',
        '2',
        '--memory',
        '2048',
        '--disk-size',
        '10',
      ],
      { stdout: 'ignore', stderr: 'pipe' },
    )

    await this.drainStderr(proc, onLog)
    const code = await proc.exited
    if (code !== 0)
      throw new Error(`podman machine init failed with code ${code}`)
  }

  async startMachine(onLog?: LogFn): Promise<void> {
    if (isLinux) return

    const proc = Bun.spawn([this.podmanPath, 'machine', 'start'], {
      stdout: 'ignore',
      stderr: 'pipe',
    })

    await this.drainStderr(proc, onLog)
    const code = await proc.exited
    if (code !== 0)
      throw new Error(`podman machine start failed with code ${code}`)
  }

  async stopMachine(): Promise<void> {
    if (isLinux) return

    const proc = Bun.spawn([this.podmanPath, 'machine', 'stop'], {
      stdout: 'ignore',
      stderr: 'ignore',
    })
    const code = await proc.exited
    if (code !== 0)
      throw new Error(`podman machine stop failed with code ${code}`)
    this.machineReady = false
  }

  async ensureReady(onLog?: LogFn): Promise<void> {
    if (this.machineReady) return

    const status = await this.getMachineStatus()

    if (!status.initialized) {
      onLog?.('Initializing Podman machine...')
      await this.initMachine(onLog)
    }

    if (!status.running) {
      onLog?.('Starting Podman machine...')
      await this.startMachine(onLog)
    }

    this.machineReady = true
  }

  async runCommand(
    args: string[],
    options?: {
      cwd?: string
      env?: Record<string, string>
      onOutput?: (line: string) => void
    },
  ): Promise<number> {
    const useStreaming = !!options?.onOutput
    const proc = Bun.spawn([this.podmanPath, ...args], {
      cwd: options?.cwd,
      env: options?.env ? { ...process.env, ...options.env } : undefined,
      stdout: useStreaming ? 'pipe' : 'ignore',
      stderr: useStreaming ? 'pipe' : 'ignore',
    })

    if (options?.onOutput) {
      await Promise.all([
        this.drainStream(proc.stdout ?? null, options.onOutput),
        this.drainStream(proc.stderr ?? null, options.onOutput),
      ])
    }

    return proc.exited
  }

  /**
   * Lists running container names. Used to check whether non-BrowserOS
   * containers are running before stopping the Podman machine.
   */
  async listRunningContainers(): Promise<string[]> {
    const proc = Bun.spawn([this.podmanPath, 'ps', '--format', '{{.Names}}'], {
      stdout: 'pipe',
      stderr: 'ignore',
    })
    const output = await new Response(proc.stdout).text()
    await proc.exited

    return output
      .trim()
      .split('\n')
      .filter((name) => name.trim())
  }

  private async drainStderr(
    proc: {
      stderr: ReadableStream<Uint8Array> | null
      exited: Promise<number>
    },
    onLog?: LogFn,
  ): Promise<void> {
    if (!onLog || !proc.stderr) return
    await this.drainStream(proc.stderr, onLog)
  }

  private async drainStream(
    stream: ReadableStream<Uint8Array> | null,
    onLine: (line: string) => void,
  ): Promise<void> {
    if (!stream) return
    const reader = stream.getReader()
    const decoder = new TextDecoder()
    let buffer = ''

    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      const lines = buffer.split('\n')
      buffer = lines.pop() ?? ''
      for (const line of lines) {
        const trimmed = line.trim()
        if (trimmed) onLine(trimmed)
      }
    }
    if (buffer.trim()) onLine(buffer.trim())
  }
}

let runtime: PodmanRuntime | null = null

export function getPodmanRuntime(): PodmanRuntime {
  if (!runtime) runtime = new PodmanRuntime()
  return runtime
}
