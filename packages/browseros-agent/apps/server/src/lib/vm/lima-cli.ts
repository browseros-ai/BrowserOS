/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { existsSync } from 'node:fs'
import { logger } from '../logger'
import { LimaCommandError, VmNotReadyError } from './errors'
import { getLimaSshConfigPath } from './paths'
import { VM_TELEMETRY_EVENTS } from './telemetry'

export interface LimaListEntry {
  name: string
  status: string
  dir: string
}

export interface LimaCliConfig {
  limactlPath: string
  limaHome: string
  sshPath?: string
}

export interface LimaShellStreams {
  onStdout?: (line: string) => void
  onStderr?: (line: string) => void
}

export interface LimaShellProcess {
  kill: () => void
  exited: Promise<number>
}

const LIMA_VERBOSE_LOGGING = false

export class LimaCli {
  constructor(private readonly cfg: LimaCliConfig) {}

  async list(): Promise<LimaListEntry[]> {
    const result = await this.run(['list', '--format', 'json'])
    if (!result.stdout.trim()) {
      logger.debug('Lima list returned no instances', {
        limaHome: this.cfg.limaHome,
      })
      return []
    }
    const entries = parseLimaList(result.stdout)
    logger.debug('Lima list parsed', {
      limaHome: this.cfg.limaHome,
      count: entries.length,
      entries: entries.map((e) => ({ name: e.name, status: e.status })),
    })
    return entries
  }

  async create(name: string, yamlPath: string): Promise<void> {
    await this.runChecked('create', [
      'create',
      '--tty=false',
      `--name=${name}`,
      yamlPath,
    ])
  }

  async start(name: string): Promise<void> {
    logger.info('Invoking limactl start', {
      vmName: name,
      limaHome: this.cfg.limaHome,
      note: 'this command blocks until boot reaches READY; may take 40-120s on first boot',
    })
    await this.runChecked('start', ['start', '--tty=false', name])
  }

  async stop(name: string): Promise<void> {
    await this.runChecked('stop', ['stop', name])
  }

  async delete(name: string): Promise<void> {
    await this.runChecked('delete', ['delete', '--force', name])
  }

  async shell(
    name: string,
    args: string[],
    streams?: LimaShellStreams,
  ): Promise<number> {
    const proc = this.spawnShell(name, args, streams)
    return proc.exited
  }

  spawnShell(
    name: string,
    args: string[],
    streams?: LimaShellStreams,
  ): LimaShellProcess {
    const configPath = getLimaSshConfigPath(this.cfg.limaHome, name)
    if (!existsSync(configPath)) {
      throw new VmNotReadyError(
        `lima ssh.config not found at ${configPath}; VM has not been started`,
      )
    }
    const proc = Bun.spawn(
      [
        this.cfg.sshPath ?? 'ssh',
        '-F',
        configPath,
        `lima-${name}`,
        shellQuoteCommand(args),
      ],
      {
        cwd: '/',
        env: this.env(),
        stdout: streams?.onStdout ? 'pipe' : 'ignore',
        stderr: streams?.onStderr ? 'pipe' : 'ignore',
      },
    )

    const drained = Promise.all([
      drainStream(proc.stdout ?? null, streams?.onStdout),
      drainStream(proc.stderr ?? null, streams?.onStderr),
    ])
    const exited = drained.then(() => proc.exited)
    return {
      exited,
      kill: () => {
        try {
          proc.kill()
        } catch {
          return
        }
      },
    }
  }

  private async runChecked(command: string, args: string[]): Promise<void> {
    const result = await this.run(args)
    if (result.exitCode !== 0) {
      throw new LimaCommandError(
        `limactl ${command}`,
        result.exitCode,
        result.stderr,
      )
    }
  }

  private async run(args: string[]): Promise<{
    exitCode: number
    stdout: string
    stderr: string
  }> {
    const started = Date.now()
    const proc = Bun.spawn([this.cfg.limactlPath, ...args], {
      env: this.env(),
      stdout: 'pipe',
      stderr: 'pipe',
    })
    logger.debug(VM_TELEMETRY_EVENTS.limaSpawn, {
      pid: proc.pid,
      args,
      limaHome: this.cfg.limaHome,
    })

    const stderrLogger = LIMA_VERBOSE_LOGGING
      ? (line: string) => {
          logger.debug(VM_TELEMETRY_EVENTS.limaStderrChunk, {
            pid: proc.pid,
            firstArg: args[0],
            line,
          })
        }
      : undefined
    const [stdout, stderr, exitCode] = await Promise.all([
      drainToString(proc.stdout),
      drainToString(proc.stderr, stderrLogger),
      proc.exited,
    ])
    const durationMs = Date.now() - started
    logger.debug(VM_TELEMETRY_EVENTS.limaExit, {
      pid: proc.pid,
      firstArg: args[0],
      exitCode,
      durationMs,
      stdoutLen: stdout.length,
      stderrLen: stderr.length,
    })
    return { exitCode, stdout, stderr }
  }

  private env(): NodeJS.ProcessEnv {
    return { ...process.env, LIMA_HOME: this.cfg.limaHome }
  }
}

async function drainToString(
  stream: ReadableStream<Uint8Array> | null,
  onLine?: (line: string) => void,
): Promise<string> {
  if (!stream) return ''
  const reader = stream.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  let output = ''
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    const chunk = decoder.decode(value, { stream: true })
    output += chunk
    buffer += chunk
    const lines = buffer.split('\n')
    buffer = lines.pop() ?? ''
    for (const line of lines) {
      const trimmed = line.trim()
      if (trimmed && onLine) onLine(trimmed)
    }
  }
  if (buffer.trim() && onLine) onLine(buffer.trim())
  return output
}

function parseLimaList(output: string): LimaListEntry[] {
  const trimmed = output.trim()
  try {
    const parsed = JSON.parse(trimmed) as unknown
    if (Array.isArray(parsed)) return parsed.map(toLimaListEntry)
    return [toLimaListEntry(parsed)]
  } catch {
    return trimmed.split('\n').map((line) => toLimaListEntry(JSON.parse(line)))
  }
}

function toLimaListEntry(input: unknown): LimaListEntry {
  const entry = input as Partial<LimaListEntry>
  return {
    name: entry.name ?? '',
    status: entry.status ?? '',
    dir: entry.dir ?? '',
  }
}

function shellQuoteCommand(args: string[]): string {
  return args.map(shellQuote).join(' ')
}

function shellQuote(arg: string): string {
  return `'${arg.replaceAll("'", "'\\''")}'`
}

async function drainStream(
  stream: ReadableStream<Uint8Array> | null,
  onLine?: (line: string) => void,
): Promise<void> {
  if (!stream || !onLine) return
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
      if (line.trim()) onLine(line.trim())
    }
  }

  if (buffer.trim()) onLine(buffer.trim())
}
