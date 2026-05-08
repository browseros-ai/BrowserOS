/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Abstract base for every container-backed agent adapter. Owns the
 * state machine, lifecycle lock, and the gated `execute*` family.
 * Subclasses provide image / spec / readiness probe / mount roots —
 * see `./types.ts` for the contract.
 *
 * Layering: this lives at the container-orchestration boundary.
 * Anything above (harness, ACP runtime, HTTP routes, UI) talks to
 * the abstraction; anything below (`ContainerCli`, `VmRuntime`,
 * `ImageLoader`) is plumbed through the constructor. The ACP layer
 * never imports `limactl`, `LIMA_HOME`, or container names again —
 * `buildExecArgv` is the single owner of that string.
 */

import { logger } from '../../logger'
import { withProcessLock } from '../../process-lock'
import { ContainerNameInUseError } from '../../vm/errors'
import type { VmRuntime } from '../../vm/vm-runtime'
import { type ContainerCli, isNoSuchContainer } from '../container-cli'
import type { ImageLoader } from '../image-loader'
import type { ContainerSpec, LogFn } from '../types'
import {
  ContainerNotReadyError,
  PathOutsideMountsError,
  ResetNotSupportedError,
} from './errors'
import type {
  ContainerDescriptor,
  ContainerState,
  ContainerStatusSnapshot,
  ExecResult,
  ExecSpec,
  MountRoot,
  ResetLevel,
  ResetOptions,
} from './types'

export interface ManagedContainerDeps {
  cli: ContainerCli
  loader: ImageLoader
  vm: VmRuntime
  /** Absolute path to the bundled `limactl`. */
  limactlPath: string
  /** `LIMA_HOME` env value to scope limactl to BrowserOS's VM dir. */
  limaHome: string
  /** Lima VM name (today: a single shared VM). */
  vmName: string
  /** Process-lock dir for serialising lifecycle ops across processes. */
  lockDir: string
}

export type StateListener = (state: ContainerState) => void
export type Unsubscribe = () => void

/** Default budget for `execProcess` / `execOneShot` to wait through
 *  a `starting` state. After this, throws `ContainerNotReadyError`
 *  with `reason: 'timeout'`. */
const DEFAULT_EXEC_GATE_TIMEOUT_MS = 60_000

export abstract class ManagedContainer {
  // ── Subclass contract ───────────────────────────────────────────
  abstract readonly descriptor: ContainerDescriptor
  protected abstract buildContainerSpec(): Promise<ContainerSpec>
  protected abstract readinessProbe(): Promise<boolean>
  protected abstract mountRoots(): readonly MountRoot[]

  // ── State ───────────────────────────────────────────────────────
  protected state: ContainerState = 'not_installed'
  protected lastError: string | null = null
  protected lastErrorAt: number | null = null
  private listeners = new Set<StateListener>()

  // Promise chain so concurrent lifecycle calls serialise within this
  // process. Cross-process serialisation lives in `withProcessLock`
  // below.
  private lifecycleLock: Promise<void> = Promise.resolve()

  constructor(protected readonly deps: ManagedContainerDeps) {}

  // ── State surface ───────────────────────────────────────────────

  getState(): ContainerState {
    return this.state
  }

  /** Returns an unsubscribe handle. Listeners fire on every transition. */
  subscribeState(listener: StateListener): Unsubscribe {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getStatusSnapshot(): ContainerStatusSnapshot {
    return {
      adapterId: this.descriptor.adapterId,
      containerName: this.descriptor.containerName,
      state: this.state,
      lastError: this.lastError,
      lastErrorAt: this.lastErrorAt,
    }
  }

  protected setState(next: ContainerState, errorMessage?: string): void {
    if (next === this.state && !errorMessage) return
    const prev = this.state
    this.state = next
    if (errorMessage !== undefined) {
      this.lastError = errorMessage
      this.lastErrorAt = Date.now()
    } else if (next === 'running') {
      // Successful reach to running clears the last error.
      this.lastError = null
      this.lastErrorAt = null
    }
    logger.debug('ManagedContainer state transition', {
      adapterId: this.descriptor.adapterId,
      from: prev,
      to: next,
      error: errorMessage,
    })
    for (const listener of this.listeners) {
      try {
        listener(next)
      } catch (err) {
        // Listener bugs must not derail the state machine.
        logger.warn('ManagedContainer state listener threw', {
          adapterId: this.descriptor.adapterId,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  // ── Lifecycle ───────────────────────────────────────────────────

  /**
   * Pull the image into the VM's containerd store. Idempotent — if
   * the image is already present, transitions through `installing →
   * installed` immediately.
   */
  async install(opts: { onLog?: (msg: string) => void } = {}): Promise<void> {
    return this.withLifecycleLock('install', async () => {
      if (this.state === 'running' || this.state === 'starting') return
      try {
        // Image ops run inside the Lima VM, so the VM has to be up
        // before nerdctl can pull. On cold boot this method is the
        // first lifecycle call to win the lock, so the ensure has to
        // happen here too — `start()` having its own ensure is not
        // enough.
        await this.deps.vm.ensureReady(opts.onLog)
        this.setState('installing')
        await this.deps.loader.ensureImageLoaded(
          this.descriptor.defaultImage,
          opts.onLog,
        )
        this.setState('installed')
      } catch (err) {
        this.setState(
          'errored',
          err instanceof Error ? err.message : String(err),
        )
        throw err
      }
    })
  }

  /**
   * Bring the container to `running`. Encapsulates: ensure VM ready,
   * ensure image, recreate container from current spec, start it,
   * wait for the daemon to report `running`, run the readiness
   * probe. Each phase transitions state so subscribers see progress.
   */
  async start(opts: { onLog?: (msg: string) => void } = {}): Promise<void> {
    return this.withLifecycleLock('start', async () => {
      if (this.state === 'running') return
      const log = (msg: string) => {
        logger.info(msg, { adapterId: this.descriptor.adapterId })
        opts.onLog?.(msg)
      }
      try {
        await this.deps.vm.ensureReady(log)
        this.setState('installing')
        await this.deps.loader.ensureImageLoaded(
          this.descriptor.defaultImage,
          log,
        )
        this.setState('starting')
        const spec = await this.buildContainerSpec()
        // Always recreate so spec changes take effect on restart. The
        // existing container (if any) is force-removed first.
        await this.deps.cli.removeContainer(spec.name, { force: true })
        await this.deps.cli.waitForContainerNameRelease(spec.name, {
          timeoutMs: 10_000,
          intervalMs: 100,
        })
        await this.deps.cli.createContainer(spec, log)
        await this.deps.cli.startContainer(spec.name, log)
        const probeOpts = this.descriptor.readinessProbe
        await this.deps.cli.waitForContainerRunning(spec.name, {
          timeoutMs: probeOpts?.timeoutMs ?? 30_000,
          intervalMs: probeOpts?.intervalMs ?? 500,
        })
        // Run the subclass-defined probe — usually a `--version` exec
        // or HTTP /readyz call. Failing this is errored, not stopped.
        const probeOk = await this.readinessProbe()
        if (!probeOk) {
          this.setState(
            'errored',
            'Readiness probe failed after container reached running state',
          )
          throw new Error(`${this.descriptor.adapterId} readiness probe failed`)
        }
        this.setState('running')
      } catch (err) {
        if (this.state !== 'errored') {
          this.setState(
            'errored',
            err instanceof Error ? err.message : String(err),
          )
        }
        throw err
      }
    })
  }

  /** Stop and remove the container. Image and per-agent data preserved. */
  async stop(): Promise<void> {
    return this.withLifecycleLock('stop', async () => {
      try {
        await this.deps.cli.removeContainer(this.descriptor.containerName, {
          force: true,
        })
        // Stop is forward-friendly even from `errored` — the user is
        // explicitly asking for a clean state.
        this.setState('stopped')
      } catch (err) {
        // A stop failure is mostly cosmetic; we still want stopped to
        // be the user's reality. Log and recover.
        logger.warn('ManagedContainer stop failed', {
          adapterId: this.descriptor.adapterId,
          error: err instanceof Error ? err.message : String(err),
        })
        this.setState('stopped')
      }
    })
  }

  async restart(opts: { onLog?: (msg: string) => void } = {}): Promise<void> {
    await this.stop()
    await this.start(opts)
  }

  /**
   * Reset is intentionally a stub here — it pins the API shape so the
   * follow-up PR can land without revving the abstract class. The
   * follow-up will implement `'soft'` (= restart), `'wipe-agent'`
   * (= soft + `rm -rf` per-agent home dir), and `'hard'` (delegated
   * to a `VmService` that hasn't shipped yet).
   */
  async reset(level: ResetLevel, _opts: ResetOptions = {}): Promise<void> {
    throw new ResetNotSupportedError(
      level,
      `reset(${level}) is not implemented yet — wired in a follow-up PR`,
    )
  }

  // ── Image / logs ────────────────────────────────────────────────

  /** True iff the existing container's image matches `descriptor.defaultImage`; treats SHA-pinned variants as a match. */
  async isImageCurrent(): Promise<boolean> {
    const actual = await this.deps.cli.containerImageRef(
      this.descriptor.containerName,
    )
    if (!actual) return false
    const expected = this.descriptor.defaultImage
    return actual === expected || actual.startsWith(`${expected}@sha256:`)
  }

  /** Tail the last `n` log lines; returns [] if the container is missing, throws on other CLI failures. */
  async getLogs(tail = 50): Promise<string[]> {
    const lines: string[] = []
    const args = ['logs', '-n', String(tail), this.descriptor.containerName]
    const result = await this.deps.cli.runCommand(args, (line) =>
      lines.push(line),
    )
    if (result.exitCode === 0) return lines
    if (isNoSuchContainer(result.stderr)) return []
    throw new Error(
      `nerdctl ${args.join(' ')} exited ${result.exitCode}: ${result.stderr.trim() || '(no stderr)'}`,
    )
  }

  /** Stream container logs until the returned handle is invoked. */
  tailLogs(onLine: LogFn): () => void {
    return this.deps.cli.tailLogs(this.descriptor.containerName, onLine)
  }

  // ── Sibling-container one-shot ──────────────────────────────────

  /** Run `argv` in a throwaway `<name>-setup` sibling using the same image+mounts; force-removes after, including on throw or timeout. */
  async runOneShot(
    argv: ReadonlyArray<string>,
    opts: {
      onLog?: LogFn
      env?: Record<string, string>
      processTimeoutMs?: number
    } = {},
  ): Promise<ExecResult> {
    return this.withLifecycleLock('run-one-shot', async () => {
      const setupName = `${this.descriptor.containerName}-setup`
      const liveSpec = await this.buildContainerSpec()
      const setupSpec: ContainerSpec = {
        ...liveSpec,
        name: setupName,
        restart: 'no',
        ports: undefined,
        health: undefined,
        env: { ...liveSpec.env, ...opts.env },
        command: [...argv] as [string, ...string[]],
      }
      try {
        await this.createOneShotContainer(setupSpec, opts.onLog)
        const result = await this.runWithOptionalTimeout(
          ['start', '-a', setupName],
          opts.onLog,
          opts.processTimeoutMs,
        )
        return {
          exitCode: result.exitCode,
          stdout: result.stdout,
          stderr: result.stderr,
        }
      } finally {
        await this.deps.cli.removeContainer(
          setupName,
          { force: true },
          opts.onLog,
        )
      }
    })
  }

  /** nerdctl occasionally lags releasing a name after `rm`; retry to absorb that. */
  private async createOneShotContainer(
    spec: ContainerSpec,
    onLog?: LogFn,
  ): Promise<void> {
    const maxAttempts = 3
    let attempt = 1
    while (true) {
      await this.deps.cli.removeContainer(spec.name, { force: true }, onLog)
      await this.deps.cli.waitForContainerNameRelease(spec.name, {
        timeoutMs: 10_000,
        intervalMs: 100,
      })
      try {
        await this.deps.cli.createContainer(spec, onLog)
        return
      } catch (err) {
        if (
          !(err instanceof ContainerNameInUseError) ||
          attempt >= maxAttempts
        ) {
          throw err
        }
        logger.warn('One-shot container name still in use; retrying', {
          adapterId: this.descriptor.adapterId,
          containerName: spec.name,
          attempt,
          maxAttempts,
        })
        attempt += 1
      }
    }
  }

  private async runWithOptionalTimeout(
    args: string[],
    onLog: LogFn | undefined,
    timeoutMs: number | undefined,
  ): Promise<ExecResult> {
    if (timeoutMs === undefined) return this.deps.cli.runCommand(args, onLog)
    // The underlying runCommand keeps streaming onLog after a timeout
    // until removeContainer kills `nerdctl start -a`. Gate the wrapped
    // callback so callers don't see stale lines after rejection.
    let active = true
    const guardedOnLog: LogFn | undefined = onLog
      ? (line) => {
          if (active) onLog(line)
        }
      : undefined
    let timer: ReturnType<typeof setTimeout> | null = null
    const timeoutPromise = new Promise<ExecResult>((_, reject) => {
      timer = setTimeout(() => {
        active = false
        reject(
          new Error(
            `One-shot exceeded timeout of ${timeoutMs}ms for ${args.join(' ')}`,
          ),
        )
      }, timeoutMs)
    })
    try {
      return await Promise.race([
        this.deps.cli.runCommand(args, guardedOnLog),
        timeoutPromise,
      ])
    } finally {
      if (timer !== null) clearTimeout(timer)
    }
  }

  // ── Execute family ──────────────────────────────────────────────

  /**
   * Build the shell-command string acpx-core would spawn for an
   * `ExecSpec`. Pure builder; does NOT gate on state. Callers that
   * need state gating (i.e. anyone other than acpx-core which has
   * its own scheduling) should use `execProcess` / `execOneShot`.
   *
   * Returned string is intended for `sh -c …` consumption — see
   * `acpx-runtime.ts` for the historical context.
   */
  buildExecArgv(spec: ExecSpec): string {
    return this.buildExecArgvArray(spec).join(' ')
  }

  /**
   * Spawn a long-lived child process inside the container, gated on
   * `state === 'running'`. Waits through `starting` up to
   * `execGateTimeoutMs` (default 60s). Returns Bun's spawned process
   * — caller owns stdio piping and `exit`.
   *
   * The ACP runtime continues to use `buildExecArgv` directly
   * because acpx-core does its own spawn; this entry point exists
   * for `execOneShot` and for callers that want to own stdio.
   */
  async execProcess(
    spec: ExecSpec,
    opts: { execGateTimeoutMs?: number } = {},
  ): Promise<Bun.Subprocess<'pipe', 'pipe', 'pipe'>> {
    await this.waitForRunning(
      opts.execGateTimeoutMs ?? DEFAULT_EXEC_GATE_TIMEOUT_MS,
    )
    const argv = this.buildExecArgvArray(spec)
    return Bun.spawn(argv, {
      stdin: 'pipe',
      stdout: 'pipe',
      stderr: 'pipe',
    })
  }

  /**
   * Convenience wrapper for short-lived commands the harness or
   * control-panel might need (cleanup, version checks, diagnostics).
   * Buffers stdio and returns once the process exits.
   */
  async execOneShot(
    spec: ExecSpec,
    opts: { execGateTimeoutMs?: number; processTimeoutMs?: number } = {},
  ): Promise<ExecResult> {
    const proc = await this.execProcess(spec, {
      execGateTimeoutMs: opts.execGateTimeoutMs,
    })
    if (opts.processTimeoutMs !== undefined) {
      const timer = setTimeout(() => {
        try {
          proc.kill()
        } catch {
          // best-effort
        }
      }, opts.processTimeoutMs)
      proc.exited.finally(() => clearTimeout(timer))
    }
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
    ])
    return { exitCode, stdout, stderr }
  }

  // ── Filesystem ──────────────────────────────────────────────────

  /**
   * Translate a host path under one of the declared mount roots into
   * the path the container sees. Throws `PathOutsideMountsError` for
   * paths that don't sit under any mount.
   */
  toContainerPath(hostPath: string): string {
    const normalized = normalizePath(hostPath)
    for (const mount of this.mountRoots()) {
      const root = normalizePath(mount.hostPath)
      if (isPathInside(root, normalized)) {
        const suffix = normalized.slice(root.length)
        return joinContainerPath(mount.containerPath, suffix)
      }
    }
    throw new PathOutsideMountsError(hostPath, 'host->container')
  }

  /** Inverse of `toContainerPath`. */
  toHostPath(containerPath: string): string {
    const normalized = normalizePath(containerPath)
    for (const mount of this.mountRoots()) {
      const root = normalizePath(mount.containerPath)
      if (isPathInside(root, normalized)) {
        const suffix = normalized.slice(root.length)
        return joinContainerPath(mount.hostPath, suffix)
      }
    }
    throw new PathOutsideMountsError(containerPath, 'container->host')
  }

  // ── Internals ───────────────────────────────────────────────────

  /**
   * Build the argv array we'd hand to `Bun.spawn` to run `spec`
   * inside the container. Pure — no state side effects. Single
   * source of truth for the limactl/nerdctl chain so the ACP layer
   * never has to know about it (closes the §4.1 leak from the
   * audit).
   */
  private buildExecArgvArray(spec: ExecSpec): string[] {
    const argv = [
      'env',
      `LIMA_HOME=${this.deps.limaHome}`,
      this.deps.limactlPath,
      'shell',
      '--workdir',
      '/',
      this.deps.vmName,
      '--',
      'nerdctl',
      'exec',
      '-i',
    ]
    for (const [key, value] of Object.entries(spec.env ?? {})) {
      argv.push('-e', `${key}=${value}`)
    }
    argv.push(this.descriptor.containerName, ...spec.argv)
    return argv
  }

  /**
   * Resolve once the container is `running`, or throw a typed
   * `ContainerNotReadyError`. `installing` / `starting` callers wait
   * up to `timeoutMs`. `not_installed` / `stopped` / `errored` reject
   * immediately — caller is expected to call `install()` / `start()`
   * / `reset()` first. Used by `execProcess`.
   */
  protected async waitForRunning(timeoutMs: number): Promise<void> {
    if (this.state === 'running') return
    if (this.state === 'not_installed') {
      throw new ContainerNotReadyError(
        this.state,
        this.descriptor.containerName,
        'not_installed',
        'Container image has not been pulled. Call install() first.',
        this.lastError,
      )
    }
    if (this.state === 'stopped') {
      throw new ContainerNotReadyError(
        this.state,
        this.descriptor.containerName,
        'stopped',
        'Container is stopped. Call start() first.',
        this.lastError,
      )
    }
    if (this.state === 'errored') {
      throw new ContainerNotReadyError(
        this.state,
        this.descriptor.containerName,
        'errored',
        'Container has hit a terminal error. Reset to recover.',
        this.lastError,
      )
    }

    // 'installing' or 'starting' — wait for either 'running' or
    // 'errored', whichever comes first, with a timeout.
    return new Promise<void>((resolve, reject) => {
      let settled = false
      const timer = setTimeout(() => {
        if (settled) return
        settled = true
        unsubscribe()
        reject(
          new ContainerNotReadyError(
            this.state,
            this.descriptor.containerName,
            'timeout',
            `Timed out after ${timeoutMs}ms waiting for container to reach running state.`,
            this.lastError,
          ),
        )
      }, timeoutMs)
      const unsubscribe = this.subscribeState((s) => {
        if (settled) return
        if (s === 'running') {
          settled = true
          clearTimeout(timer)
          unsubscribe()
          resolve()
        } else if (s === 'errored') {
          settled = true
          clearTimeout(timer)
          unsubscribe()
          reject(
            new ContainerNotReadyError(
              'errored',
              this.descriptor.containerName,
              'errored',
              'Container hit a terminal error while we were waiting.',
              this.lastError,
            ),
          )
        }
      })
    })
  }

  /**
   * Serialise lifecycle operations both within this process (via the
   * promise chain) and across processes (via file-lock).
   */
  private async withLifecycleLock<T>(
    operation: string,
    fn: () => Promise<T>,
  ): Promise<T> {
    const previous = this.lifecycleLock
    let release!: () => void
    this.lifecycleLock = new Promise<void>((resolve) => {
      release = resolve
    })
    await previous.catch(() => undefined)
    try {
      return await withProcessLock(
        `${this.descriptor.adapterId}-lifecycle`,
        { lockDir: this.deps.lockDir },
        async () => {
          logger.debug('ManagedContainer lifecycle op started', {
            adapterId: this.descriptor.adapterId,
            operation,
          })
          return await fn()
        },
      )
    } finally {
      release()
    }
  }
}

// ── Path helpers (kept private to this module) ──────────────────────

function normalizePath(p: string): string {
  // Strip trailing slashes, collapse repeats. Leave leading slash
  // alone so absolute paths stay absolute.
  let out = p.replace(/\/+/g, '/')
  if (out.length > 1 && out.endsWith('/')) out = out.slice(0, -1)
  return out
}

function isPathInside(root: string, candidate: string): boolean {
  if (candidate === root) return true
  return candidate.startsWith(`${root}/`)
}

function joinContainerPath(base: string, suffix: string): string {
  if (!suffix) return base
  if (suffix.startsWith('/')) return `${base}${suffix}`
  return `${base}/${suffix}`
}
