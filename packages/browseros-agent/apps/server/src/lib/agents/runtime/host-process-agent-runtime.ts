/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Abstract base for host-process agent runtimes (claude, codex). The
 * agent process runs from the user's host PATH — no container, no
 * Lima. This class owns binary discovery, version probing with
 * caching, and the smaller capability surface that host adapters
 * support.
 */

import { logger } from '../../logger'
import type { AgentRuntime } from './agent-runtime'
import { ActionNotSupportedError } from './errors'
import type {
  ExecSpec,
  RuntimeAction,
  RuntimeCapability,
  RuntimeDescriptor,
  RuntimeState,
  RuntimeStatusSnapshot,
  StateListener,
  Unsubscribe,
} from './types'

export interface HostProcessAgentRuntimeDeps {
  /** Host PATH binary name to probe + spawn (e.g. 'claude', 'codex'). */
  binaryName: string
  /** Override the default `<binary> --version` probe argv. */
  versionProbeArgs?: ReadonlyArray<string>
  /** Cache window for probe results in ms. Default 5 minutes — same
   *  as today's adapter-health.ts. */
  probeCacheMs?: number
  /** Test seam: spawn the probe via this fn instead of `Bun.$`. */
  spawnProbe?: (
    cmd: ReadonlyArray<string>,
    timeoutMs: number,
  ) => Promise<{ exitCode: number; stdout: string; stderr: string }>
}

const DEFAULT_PROBE_CACHE_MS = 5 * 60 * 1000
const DEFAULT_PROBE_TIMEOUT_MS = 2_000

export abstract class HostProcessAgentRuntime implements AgentRuntime {
  abstract readonly descriptor: RuntimeDescriptor & { kind: 'host-process' }
  abstract getPerAgentHomeDir(agentId: string): string

  protected state: RuntimeState = 'cli_missing'
  protected lastError: string | null = null
  protected lastErrorAt: number | null = null
  protected binaryVersion: string | null = null
  private readonly listeners = new Set<StateListener>()
  private healthCheckedAt = 0

  constructor(protected readonly deps: HostProcessAgentRuntimeDeps) {}

  // ── Status surface ───────────────────────────────────────────────

  getStatusSnapshot(): RuntimeStatusSnapshot {
    return {
      adapterId: this.descriptor.adapterId,
      state: this.state,
      isReady: this.state === 'cli_present',
      lastError: this.lastError,
      lastErrorAt: this.lastErrorAt,
      details: { binaryVersion: this.binaryVersion },
    }
  }

  subscribe(listener: StateListener): Unsubscribe {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getCapabilities(): ReadonlyArray<RuntimeCapability> {
    return ['reinstall-cli', 'check-auth']
  }

  // ── Action dispatch ──────────────────────────────────────────────

  async executeAction(
    action: RuntimeAction,
    _opts: { onLog?: (msg: string) => void } = {},
  ): Promise<void> {
    if (!this.getCapabilities().includes(action.type as RuntimeCapability)) {
      throw new ActionNotSupportedError(
        this.descriptor.adapterId,
        action.type,
        this.getCapabilities(),
      )
    }
    switch (action.type) {
      case 'reinstall-cli':
        return this.handleReinstallCli()
      case 'check-auth':
        return this.checkAuth()
      default:
        throw new ActionNotSupportedError(
          this.descriptor.adapterId,
          action.type,
          this.getCapabilities(),
        )
    }
  }

  // ── ACP plane integration ────────────────────────────────────────

  buildExecArgv(spec: ExecSpec): string {
    // Host binary lives on $PATH — no limactl chain. Compose
    // `env KEY=val ... <argv...>` so adapters that inject env
    // (AGENT_HOME, CODEX_HOME) get them on the spawned process.
    const envParts = Object.entries(spec.env ?? {}).map(([k, v]) => `${k}=${v}`)
    const prefix = envParts.length > 0 ? `env ${envParts.join(' ')} ` : ''
    return `${prefix}${spec.argv.join(' ')}`
  }

  // ── Health probe ─────────────────────────────────────────────────

  /**
   * Probe `<binary> --version` (override via deps.versionProbeArgs).
   * Cached for `probeCacheMs`. Updates state + binaryVersion +
   * fires subscribers. Idempotent within the cache window.
   */
  async probeHealth(force = false): Promise<void> {
    const cacheMs = this.deps.probeCacheMs ?? DEFAULT_PROBE_CACHE_MS
    const now = Date.now()
    if (!force && now - this.healthCheckedAt < cacheMs) return

    const argv = this.deps.versionProbeArgs ?? [
      this.deps.binaryName,
      '--version',
    ]
    try {
      const result = await this.runProbe(argv, DEFAULT_PROBE_TIMEOUT_MS)
      // Only stamp the cache once a definitive result lands. A spawn
      // throw (binary missing, perm denied) leaves the timestamp
      // untouched so the next call re-probes immediately.
      this.healthCheckedAt = Date.now()
      if (result.exitCode === 0) {
        this.binaryVersion = result.stdout.trim() || null
        this.setState('cli_present')
      } else {
        this.binaryVersion = null
        this.setState(
          'cli_unhealthy',
          `${this.deps.binaryName} --version exited ${result.exitCode}: ${result.stderr.trim() || '(no stderr)'}`,
        )
      }
    } catch (err) {
      // Spawn failure (binary missing, permission denied) — distinct
      // from a non-zero exit, so the UI can render different copy.
      this.binaryVersion = null
      this.setState(
        'cli_missing',
        err instanceof Error ? err.message : String(err),
      )
    }
  }

  // ── Subclass hooks ───────────────────────────────────────────────

  /** Subclass override — claude reads ~/.claude/auth.json, codex
   *  reads <CODEX_HOME>/auth.json, etc. Default is a no-op. */
  protected async checkAuth(): Promise<void> {
    return
  }

  /** Default reinstall-cli handler — throws an informative error
   *  pointing at the upstream docs. Subclasses can override to
   *  trigger an in-app installer. */
  protected async handleReinstallCli(): Promise<void> {
    throw new Error(
      `${this.descriptor.displayName} CLI is not installed. ` +
        `Install ${this.deps.binaryName} from the upstream docs and probe again.`,
    )
  }

  // ── Internals ────────────────────────────────────────────────────

  protected setState(next: RuntimeState, errorMessage?: string): void {
    if (next === this.state && !errorMessage) return
    this.state = next
    if (errorMessage !== undefined) {
      this.lastError = errorMessage
      this.lastErrorAt = Date.now()
    } else if (next === 'cli_present') {
      this.lastError = null
      this.lastErrorAt = null
    }
    const snapshot = this.getStatusSnapshot()
    for (const listener of this.listeners) {
      try {
        listener(snapshot)
      } catch (err) {
        logger.warn('HostProcessAgentRuntime state listener threw', {
          adapterId: this.descriptor.adapterId,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  private async runProbe(
    cmd: ReadonlyArray<string>,
    timeoutMs: number,
  ): Promise<{ exitCode: number; stdout: string; stderr: string }> {
    if (this.deps.spawnProbe) return this.deps.spawnProbe(cmd, timeoutMs)
    const proc = Bun.spawn(cmd as string[], {
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const timer = setTimeout(() => {
      try {
        proc.kill()
      } catch {
        // best-effort
      }
    }, timeoutMs)
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
    ])
    clearTimeout(timer)
    return { exitCode, stdout, stderr }
  }
}
