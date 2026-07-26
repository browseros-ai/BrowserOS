/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentRuntime } from './agent-runtime'

export class AgentRuntimeRegistry {
  private readonly runtimes = new Map<string, AgentRuntime>()

  register(runtime: AgentRuntime): void {
    const id = runtime.descriptor.adapterId
    if (this.runtimes.has(id)) {
      throw new Error(`Runtime for adapter "${id}" is already registered`)
    }
    this.runtimes.set(id, runtime)
  }

  get(adapterId: string): AgentRuntime | null {
    return this.runtimes.get(adapterId) ?? null
  }

  list(): ReadonlyArray<AgentRuntime> {
    return Array.from(this.runtimes.values())
  }

  unregister(adapterId: string): boolean {
    return this.runtimes.delete(adapterId)
  }
}

let globalRegistry: AgentRuntimeRegistry | null = null

export function getAgentRuntimeRegistry(): AgentRuntimeRegistry {
  if (!globalRegistry) globalRegistry = new AgentRuntimeRegistry()
  return globalRegistry
}

/** Test-only — production code never calls this. */
export function resetAgentRuntimeRegistry(): void {
  globalRegistry = null
}
