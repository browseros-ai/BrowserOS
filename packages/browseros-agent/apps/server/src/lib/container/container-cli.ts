/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ContainerCliError } from '../vm/errors'
import { LimaCli } from '../vm/lima-cli'
import type { ContainerSpec, LogFn, MountSpec, PortMapping } from './types'

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

  async pullImage(ref: string, onLog?: LogFn): Promise<void> {
    await this.runRequired(['pull', ref], onLog)
  }

  async loadImage(tarballPath: string, onLog?: LogFn): Promise<string[]> {
    const result = await this.runRequired(['load', '-i', tarballPath], onLog)
    return parseLoadedImageRefs(result.stdout)
  }

  async createContainer(spec: ContainerSpec, onLog?: LogFn): Promise<void> {
    await this.runRequired(buildCreateArgs(spec), onLog)
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

function parseLoadedImageRefs(stdout: string): string[] {
  return stdout
    .split('\n')
    .map((line) => line.match(/^Loaded image(?:\(s\))?:\s*(.+)$/i)?.[1]?.trim())
    .filter((ref): ref is string => !!ref)
}

function isNoSuchContainer(stderr: string): boolean {
  const lower = stderr.toLowerCase()
  return lower.includes('no such container') || lower.includes('not found')
}

function linesToOutput(lines: string[]): string {
  if (lines.length === 0) return ''
  return `${lines.join('\n')}\n`
}
