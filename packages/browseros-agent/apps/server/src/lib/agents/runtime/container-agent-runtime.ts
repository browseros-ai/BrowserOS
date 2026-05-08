/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Abstract base for container-backed agent runtimes (openclaw,
 * hermes). Extends `ManagedContainer` so subclasses keep all the
 * existing container plumbing (state machine, lifecycle lock, image
 * load, mount roots, exec gating); adds the runtime-layer surface on
 * top: descriptor, capability list, action dispatcher, status
 * snapshot.
 */

import type {
  ContainerDescriptor,
  ContainerStatusSnapshot,
} from '../../container/managed'
import { ManagedContainer } from '../../container/managed'
import type { AgentRuntime } from './agent-runtime'
import { ActionNotSupportedError } from './errors'
import type {
  RuntimeAction,
  RuntimeCapability,
  StateListener,
  Unsubscribe,
} from './types'

export abstract class ContainerAgentRuntime
  extends ManagedContainer
  implements AgentRuntime
{
  abstract override readonly descriptor: ContainerDescriptor & {
    kind: 'container'
  }
  abstract getPerAgentHomeDir(agentId: string): string

  /**
   * Default capability list. Subclasses extend (e.g. OpenClaw adds
   * `'gateway-control-plane'`) or filter (e.g. drop reset levels the
   * subclass can't support yet).
   */
  getCapabilities(): ReadonlyArray<RuntimeCapability> {
    return [
      'install',
      'start',
      'stop',
      'restart',
      'reset-soft',
      'reset-wipe-agent',
      'reset-hard',
      'logs',
    ]
  }

  override getStatusSnapshot(): ContainerStatusSnapshot & {
    isReady: boolean
  } {
    const state = this.getState()
    return {
      adapterId: this.descriptor.adapterId,
      containerName: this.descriptor.containerName,
      state,
      isReady: state === 'running',
      lastError: this.lastError,
      lastErrorAt: this.lastErrorAt,
    }
  }

  subscribe(listener: StateListener): Unsubscribe {
    return this.subscribeState(() => listener(this.getStatusSnapshot()))
  }

  async executeAction(
    action: RuntimeAction,
    opts: { onLog?: (msg: string) => void } = {},
  ): Promise<void> {
    const required = actionToCapability(action)
    if (!this.getCapabilities().includes(required)) {
      throw new ActionNotSupportedError(
        this.descriptor.adapterId,
        action.type,
        this.getCapabilities(),
      )
    }
    switch (action.type) {
      case 'install':
        return this.install(opts)
      case 'start':
        return this.start(opts)
      case 'stop':
        return this.stop()
      case 'restart':
        return this.restart(opts)
      case 'reset-soft':
        return this.reset('soft', opts)
      case 'reset-wipe-agent':
        return this.reset('wipe-agent', { ...opts, agentId: action.agentId })
      case 'reset-hard':
        return this.reset('hard', opts)
      default:
        throw new ActionNotSupportedError(
          this.descriptor.adapterId,
          (action as { type: string }).type,
          this.getCapabilities(),
        )
    }
  }
}

/**
 * Map an action variant to the capability key the gate checks. Kept
 * outside the class so the dispatcher can guard before constructing
 * any state.
 */
function actionToCapability(action: RuntimeAction): RuntimeCapability {
  // The action.type strings happen to coincide with capability
  // strings 1:1, so this is currently identity. Pulled out as a
  // function so the gate can grow more nuanced (e.g. action-specific
  // sub-capabilities) without re-flowing the dispatcher.
  return action.type as RuntimeCapability
}
