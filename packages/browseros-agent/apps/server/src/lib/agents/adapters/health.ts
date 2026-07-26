/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentAdapter } from '../agent-types'
import { isHostAcpAdapter } from '../host-acp/config'
import {
  type AdapterHealth,
  type DetectHostAdapterOptions,
  detectHostAdapter,
} from '../host-acp/detection'
import {
  type AgentRuntime,
  type AgentRuntimeRegistry,
  getAgentRuntimeRegistry,
  HostProcessAgentRuntime,
} from '../runtime'

/** Reports adapter readiness for the `/adapters` route. */
export class AdapterHealthChecker {
  private readonly registry: AgentRuntimeRegistry
  private readonly detectHostAdapter: typeof detectHostAdapter
  private readonly hostDetectionOptions: DetectHostAdapterOptions

  constructor(
    options: {
      registry?: AgentRuntimeRegistry
      detectHostAdapter?: typeof detectHostAdapter
      hostDetectionOptions?: DetectHostAdapterOptions
    } = {},
  ) {
    this.registry = options.registry ?? getAgentRuntimeRegistry()
    this.detectHostAdapter = options.detectHostAdapter ?? detectHostAdapter
    this.hostDetectionOptions = options.hostDetectionOptions ?? {}
  }

  async getHealth(adapter: AgentAdapter): Promise<AdapterHealth> {
    if (isHostAcpAdapter(adapter)) {
      return this.detectHostAdapter(adapter, this.hostDetectionOptions)
    }

    const runtime = this.registry.get(adapter)
    if (!runtime) {
      return {
        healthy: false,
        reason: `No runtime registered for "${adapter}"`,
        checkedAt: Date.now(),
        readiness: 'needs-install',
        installState: 'not-installed',
        nativeCliState: 'unknown',
        authState: 'unknown',
        adapterLaunchSource: 'none',
        packageCacheState: 'unknown',
      }
    }
    if (runtime instanceof HostProcessAgentRuntime) await runtime.probeHealth()
    return runtimeSnapshotToHealth(runtime)
  }
}

function runtimeSnapshotToHealth(runtime: AgentRuntime): AdapterHealth {
  const snap = runtime.getStatusSnapshot()
  return {
    healthy: snap.isReady,
    reason: snap.isReady ? undefined : (snap.lastError ?? undefined),
    checkedAt: snap.probedAt ?? snap.lastErrorAt ?? Date.now(),
    readiness: snap.isReady ? 'ready' : 'unknown',
    installState: snap.isReady ? 'installed' : 'not-installed',
    nativeCliState: 'unknown',
    authState: snap.isReady ? 'not-applicable' : 'unknown',
    adapterLaunchSource: snap.isReady ? 'runtime' : 'none',
    packageCacheState: 'unknown',
  }
}
