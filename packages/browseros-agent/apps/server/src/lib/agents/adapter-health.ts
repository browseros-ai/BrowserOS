/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentAdapter } from './agent-types'
import {
  type AgentRuntime,
  type AgentRuntimeRegistry,
  getAgentRuntimeRegistry,
  HostProcessAgentRuntime,
} from './runtime'

export interface AdapterHealth {
  healthy: boolean
  /** Human-readable explanation when unhealthy; absent on success. */
  reason?: string
  /** Wall-clock ms when this probe completed. */
  checkedAt: number
}

/**
 * Reports adapter readiness for the `/adapters` route. Reads from the
 * `AgentRuntimeRegistry` — host-process runtimes self-cache their
 * `<binary> --version` probe; container runtimes expose lifecycle
 * state via the same snapshot.
 *
 * OpenClaw still falls back to a permissive default until Phase 4
 * migrates it onto a runtime — its health currently comes from the
 * gateway lifecycle snapshot the harness already exposes.
 */
export class AdapterHealthChecker {
  private readonly registry: AgentRuntimeRegistry

  constructor(options: { registry?: AgentRuntimeRegistry } = {}) {
    this.registry = options.registry ?? getAgentRuntimeRegistry()
  }

  async getHealth(adapter: AgentAdapter): Promise<AdapterHealth> {
    const runtime = this.registry.get(adapter)
    if (!runtime) return openclawFallback(adapter)
    if (runtime instanceof HostProcessAgentRuntime) await runtime.probeHealth()
    return runtimeSnapshotToHealth(runtime)
  }
}

function runtimeSnapshotToHealth(runtime: AgentRuntime): AdapterHealth {
  const snap = runtime.getStatusSnapshot()
  return {
    healthy: snap.isReady,
    reason: snap.isReady ? undefined : (snap.lastError ?? undefined),
    // Prefer probedAt so the timestamp reflects probe completion
    // regardless of health state. lastErrorAt is the fallback for
    // runtimes that don't emit probedAt yet (containers).
    checkedAt: snap.probedAt ?? snap.lastErrorAt ?? Date.now(),
  }
}

function openclawFallback(adapter: AgentAdapter): AdapterHealth {
  if (adapter === 'openclaw') return { healthy: true, checkedAt: Date.now() }
  return {
    healthy: false,
    reason: `No runtime registered for "${adapter}"`,
    checkedAt: Date.now(),
  }
}
