/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { posix, win32 } from 'node:path'

export interface HostCommandResult {
  exitCode: number
  stdout: string
  stderr: string
}

export interface HostCommandOptions {
  env?: NodeJS.ProcessEnv
  timeoutMs?: number
}

export type HostCommandRunner = (
  cmd: string,
  args: string[],
  options: HostCommandOptions,
) => Promise<HostCommandResult>

export interface ResolvedHostBinary {
  path: string
  env: NodeJS.ProcessEnv
}

export interface ResolveHostBinaryOptions {
  env?: NodeJS.ProcessEnv
  platform?: NodeJS.Platform
  timeoutMs?: number
  runCommand?: HostCommandRunner
}

const DEFAULT_LOOKUP_TIMEOUT_MS = 3_000
const SAFE_BINARY_NAME = /^[A-Za-z0-9._-]+(?:\.(?:cmd|exe|bat))?$/

/** Resolves a host command through the user's OS lookup path without hardcoded install directories. */
export async function resolveHostBinary(
  name: string,
  options: ResolveHostBinaryOptions = {},
): Promise<ResolvedHostBinary | null> {
  assertSafeBinaryName(name)
  const platform = options.platform ?? process.platform
  const env = options.env ?? process.env
  const timeoutMs = options.timeoutMs ?? DEFAULT_LOOKUP_TIMEOUT_MS
  const runCommand = options.runCommand ?? runHostCommand

  const resolvedPath =
    platform === 'win32'
      ? await resolveWindowsBinary(name, env, timeoutMs, runCommand)
      : await resolveUnixBinary(name, env, timeoutMs, runCommand)

  if (!resolvedPath) return null
  return {
    path: resolvedPath,
    env: buildResolvedBinaryEnv({ binaryPath: resolvedPath, env, platform }),
  }
}

/** Builds child process env so npm/node shims can find their shebang interpreter. */
export function buildResolvedBinaryEnv(input: {
  binaryPath: string
  env?: NodeJS.ProcessEnv
  platform?: NodeJS.Platform
}): NodeJS.ProcessEnv {
  const platform = input.platform ?? process.platform
  const env = { ...(input.env ?? process.env) }
  const key = pathEnvKey(env, platform)
  const delimiter = pathDelimiter(platform)
  const dir =
    platform === 'win32'
      ? win32.dirname(input.binaryPath)
      : posix.dirname(input.binaryPath)
  const existing = env[key] ?? ''
  const parts = existing.split(delimiter).filter(Boolean)
  env[key] = parts.includes(dir)
    ? existing
    : [dir, ...parts].filter(Boolean).join(delimiter)
  return env
}

export async function runHostCommand(
  cmd: string,
  args: string[],
  options: HostCommandOptions = {},
): Promise<HostCommandResult> {
  const proc = Bun.spawn([cmd, ...args], {
    stdin: 'ignore',
    stdout: 'pipe',
    stderr: 'pipe',
    env: options.env,
  })
  let timedOut = false
  const timer =
    options.timeoutMs && options.timeoutMs > 0
      ? setTimeout(() => {
          timedOut = true
          try {
            proc.kill()
          } catch {
            // best effort
          }
        }, options.timeoutMs)
      : null

  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ])
  if (timer) clearTimeout(timer)
  if (timedOut)
    throw new Error(`Command timed out after ${options.timeoutMs}ms`)
  return { exitCode, stdout, stderr }
}

async function resolveUnixBinary(
  name: string,
  env: NodeJS.ProcessEnv,
  timeoutMs: number,
  runCommand: HostCommandRunner,
): Promise<string | null> {
  const shell = env.SHELL?.trim()
  if (shell) {
    const viaLoginShell = await lookupWithCommand(
      shell,
      ['-lic', `command -v ${name}`],
      env,
      timeoutMs,
      runCommand,
    )
    if (viaLoginShell) return viaLoginShell
  }

  return lookupWithCommand(
    'sh',
    ['-lc', `command -v ${name}`],
    env,
    timeoutMs,
    runCommand,
  )
}

async function resolveWindowsBinary(
  name: string,
  env: NodeJS.ProcessEnv,
  timeoutMs: number,
  runCommand: HostCommandRunner,
): Promise<string | null> {
  const viaWhere = await lookupWithCommand(
    'where.exe',
    [name],
    env,
    timeoutMs,
    runCommand,
  )
  if (viaWhere) return viaWhere

  return lookupWithCommand(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      `(Get-Command -CommandType Application ${name} -ErrorAction SilentlyContinue).Source`,
    ],
    env,
    timeoutMs,
    runCommand,
  )
}

async function lookupWithCommand(
  cmd: string,
  args: string[],
  env: NodeJS.ProcessEnv,
  timeoutMs: number,
  runCommand: HostCommandRunner,
): Promise<string | null> {
  const result = await runCommand(cmd, args, { env, timeoutMs }).catch(
    () => null,
  )
  if (result?.exitCode !== 0) return null
  return firstOutputPath(result.stdout)
}

function firstOutputPath(stdout: string): string | null {
  const line = stdout
    .split(/\r?\n/)
    .map((part) => part.trim())
    .find(Boolean)
  return line && line.length > 0 ? line : null
}

function assertSafeBinaryName(name: string): void {
  if (!SAFE_BINARY_NAME.test(name)) {
    throw new Error(`Unsafe binary name: ${name}`)
  }
}

function pathDelimiter(platform: NodeJS.Platform): string {
  return platform === 'win32' ? ';' : ':'
}

function pathEnvKey(env: NodeJS.ProcessEnv, platform: NodeJS.Platform): string {
  if (platform !== 'win32') return 'PATH'
  return Object.keys(env).find((key) => key.toLowerCase() === 'path') ?? 'Path'
}
