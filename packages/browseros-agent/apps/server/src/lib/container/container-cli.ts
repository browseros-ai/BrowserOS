/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  ContainerCliError,
  ContainerNameInUseError,
  ContainerNameReleaseTimeoutError,
  ContainerNotRunningError,
} from '../vm/errors'
import { LimaCli } from '../vm/lima-cli'
import type {
  ContainerInfo,
  ContainerSpec,
  LogFn,
  MountSpec,
  PortMapping,
  WaitForContainerNameReleaseOptions,
  WaitForContainerRunningOptions,
} from './types'

export function buildNerdctlCommand(args: string[]): string[] {
  return ['nerdctl', ...args]
}

export interface ContainerCliConfig {
  limactlPath: string
  limaHome: string
  vmName: string
  sshPath?: string
}

export interface ContainerCommandResult {
  exitCode: number
  stdout: string
  stderr: string
}

export class ContainerCli {
  private readonly lima: LimaCli

  constructor(private readonly cfg: ContainerCliConfig) {
    this.lima = new LimaCli({
      limactlPath: cfg.limactlPath,
      limaHome: cfg.limaHome,
      sshPath: cfg.sshPath,
    })
  }

  async imageExists(ref: string): Promise<boolean> {
    const result = await this.runCommand(['image', 'inspect', ref])
    return result.exitCode === 0
  }

  /** Return the image ref used to create a container, or null when absent. */
  async containerImageRef(name: string): Promise<string | null> {
    const args = ['inspect', '--format', '{{.Config.Image}}', name]
    const result = await this.runCommand(args)
    if (result.exitCode === 0) {
      const image = result.stdout.trim()
      return image || null
    }
    if (isNoSuchContainer(result.stderr)) return null
    throw this.commandError(args, result)
  }

  async pullImage(ref: string, onLog?: LogFn): Promise<void> {
    await this.runRequired(['pull', ref], onLog)
  }

  async createContainer(spec: ContainerSpec, onLog?: LogFn): Promise<void> {
    const args = buildCreateArgs(spec)
    const result = await this.runCommand(args, onLog)
    if (result.exitCode === 0) return
    if (isContainerNameInUse(result.stderr)) {
      throw new ContainerNameInUseError(
        spec.name,
        `nerdctl ${args.join(' ')}`,
        result.exitCode,
        result.stderr.trim(),
      )
    }
    throw this.commandError(args, result)
  }

  async startContainer(name: string, onLog?: LogFn): Promise<void> {
    await this.runRequired(['start', name], onLog)
  }

  async stopContainer(name: string, onLog?: LogFn): Promise<void> {
    const result = await this.runCommand(['stop', name], onLog)
    if (result.exitCode === 0 || isNoSuchContainer(result.stderr)) return
    throw this.commandError(['stop', name], result)
  }

  async removeContainer(
    name: string,
    opts?: { force?: boolean },
    onLog?: LogFn,
  ): Promise<void> {
    const args = ['rm']
    if (opts?.force) args.push('-f')
    args.push(name)
    const result = await this.runCommand(args, onLog)
    if (result.exitCode === 0 || isNoSuchContainer(result.stderr)) return
    throw this.commandError(args, result)
  }

  /** Inspect a named container without treating absence as a command failure. */
  async inspectContainer(name: string): Promise<ContainerInfo | null> {
    const args = ['container', 'inspect', '--format', '{{json .}}', name]
    const result = await this.runCommand(args)
    if (result.exitCode === 0) {
      return parseContainerInfo(result.stdout, name)
    }
    if (isNoSuchContainer(result.stderr)) return null
    throw this.commandError(args, result)
  }

  /**
   * Poll `nerdctl container inspect` until the container reports
   * `running: true`. Used by the managed-container layer between
   * `nerdctl create + start` and "container is ready for `exec`" so
   * the harness never spawns a turn against a half-started container.
   *
   * Distinct from `waitForContainerNameRelease`, which waits for the
   * container to disappear after `rm`. Defaults are sized for a
   * cold-start: 30 s budget at 500 ms cadence (60 polls). Caller can
   * tighten for tests via `opts`.
   */
  async waitForContainerRunning(
    name: string,
    opts: WaitForContainerRunningOptions = {},
  ): Promise<void> {
    const timeoutMs = opts.timeoutMs ?? 30_000
    const intervalMs = opts.intervalMs ?? 500
    const startedAt = Date.now()

    while (Date.now() - startedAt <= timeoutMs) {
      const info = await this.inspectContainer(name)
      if (info?.running === true) return
      const remainingMs = timeoutMs - (Date.now() - startedAt)
      if (remainingMs <= 0) break
      await Bun.sleep(Math.min(intervalMs, remainingMs))
    }

    throw new ContainerNotRunningError(name, timeoutMs)
  }

  /** Wait for containerd/nerdctl to stop resolving a container name after rm. */
  async waitForContainerNameRelease(
    name: string,
    opts: WaitForContainerNameReleaseOptions = {},
  ): Promise<void> {
    const timeoutMs = opts.timeoutMs ?? 5_000
    const intervalMs = opts.intervalMs ?? 100
    const startedAt = Date.now()

    while (Date.now() - startedAt <= timeoutMs) {
      if (!(await this.inspectContainer(name))) return
      const remainingMs = timeoutMs - (Date.now() - startedAt)
      if (remainingMs <= 0) break
      await Bun.sleep(Math.min(intervalMs, remainingMs))
    }

    throw new ContainerNameReleaseTimeoutError(name, timeoutMs)
  }

  async exec(name: string, cmd: string[], onLog?: LogFn): Promise<number> {
    const result = await this.runCommand(['exec', name, ...cmd], onLog)
    return result.exitCode
  }

  async ps(opts?: { namesOnly?: boolean }): Promise<string[]> {
    const args = opts?.namesOnly ? ['ps', '--format', '{{.Names}}'] : ['ps']
    const result = await this.runRequired(args)
    return result.stdout
      .trim()
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
  }

  tailLogs(name: string, onLine: LogFn): () => void {
    const proc = this.lima.spawnShell(
      this.cfg.vmName,
      buildNerdctlCommand(['logs', '-f', '-n', '0', name]),
      { onStdout: onLine, onStderr: onLine },
    )

    let stopped = false
    return () => {
      if (stopped) return
      stopped = true
      proc.kill()
    }
  }

  async runCommand(
    args: string[],
    onLog?: LogFn,
  ): Promise<ContainerCommandResult> {
    const stdoutLines: string[] = []
    const stderrLines: string[] = []
    const exitCode = await this.lima.shell(
      this.cfg.vmName,
      buildNerdctlCommand(args),
      {
        onStdout: (line) => {
          stdoutLines.push(line)
          onLog?.(line)
        },
        onStderr: (line) => {
          stderrLines.push(line)
          onLog?.(line)
        },
      },
    )

    return {
      exitCode,
      stdout: linesToOutput(stdoutLines),
      stderr: stderrLines.join('\n'),
    }
  }

  private async runRequired(
    args: string[],
    onLog?: LogFn,
  ): Promise<ContainerCommandResult> {
    const result = await this.runCommand(args, onLog)
    if (result.exitCode === 0) return result
    throw this.commandError(args, result)
  }

  private commandError(
    args: string[],
    result: ContainerCommandResult,
  ): ContainerCliError {
    return new ContainerCliError(
      `nerdctl ${args.join(' ')}`,
      result.exitCode,
      result.stderr.trim(),
    )
  }
}

function buildCreateArgs(spec: ContainerSpec): string[] {
  const args = ['create', '--name', spec.name]

  if (spec.restart) args.push('--restart', spec.restart)
  for (const port of spec.ports ?? []) args.push('-p', portArg(port))
  if (spec.envFile) args.push('--env-file', spec.envFile)
  for (const [key, value] of Object.entries(spec.env ?? {})) {
    args.push('-e', `${key}=${value}`)
  }
  for (const mount of spec.mounts ?? []) args.push('-v', mountArg(mount))
  for (const host of spec.addHosts ?? []) args.push('--add-host', host)
  if (spec.health) {
    args.push('--health-cmd', spec.health.cmd)
    if (spec.health.interval)
      args.push('--health-interval', spec.health.interval)
    if (spec.health.timeout) args.push('--health-timeout', spec.health.timeout)
    if (spec.health.retries !== undefined) {
      args.push('--health-retries', String(spec.health.retries))
    }
  }
  if (spec.entrypoint) args.push('--entrypoint', spec.entrypoint)

  args.push(spec.image)
  args.push(...(spec.command ?? []))
  return args
}

function portArg(port: PortMapping): string {
  const host = port.hostIp ? `${port.hostIp}:${port.hostPort}` : port.hostPort
  return `${host}:${port.containerPort}`
}

function mountArg(mount: MountSpec): string {
  return `${mount.source}:${mount.target}${mount.readonly ? ':ro' : ''}`
}

function parseContainerInfo(
  stdout: string,
  fallbackName: string,
): ContainerInfo {
  const line = stdout
    .trim()
    .split('\n')
    .map((entry) => entry.trim())
    .find(Boolean)
  if (!line) {
    throw new Error(`nerdctl container inspect returned empty output`)
  }
  const parsed = JSON.parse(line) as unknown
  const container = Array.isArray(parsed) ? parsed[0] : parsed
  const object = isRecord(container) ? container : {}
  const config = isRecord(object.Config) ? object.Config : {}
  const state = isRecord(object.State) ? object.State : {}
  const name = stringValue(object.Name)?.replace(/^\/+/, '') ?? fallbackName
  const status = stringValue(state.Status) ?? stringValue(object.Status)
  const running =
    typeof state.Running === 'boolean'
      ? state.Running
      : status
        ? status.toLowerCase() === 'running'
        : null

  return {
    id: stringValue(object.ID) ?? stringValue(object.Id),
    name,
    image: stringValue(config.Image) ?? stringValue(object.Image),
    status,
    running,
  }
}

export function isNoSuchContainer(stderr: string): boolean {
  const lower = stderr.toLowerCase()
  return (
    lower.includes('no such container') || lower.includes('container not found')
  )
}

export function isContainerNameInUse(stderr: string): boolean {
  const lower = stderr.toLowerCase()
  return (
    (lower.includes('name-store error') && lower.includes('already used')) ||
    lower.includes('name is already in use')
  )
}

function linesToOutput(lines: string[]): string {
  if (lines.length === 0) return ''
  return `${lines.join('\n')}\n`
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function stringValue(value: unknown): string | null {
  return typeof value === 'string' && value ? value : null
}
