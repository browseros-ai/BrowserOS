/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export type { AgentRuntime } from './agent-runtime'
export { ContainerAgentRuntime } from './container-agent-runtime'
export { ActionNotSupportedError, RuntimeNotReadyError } from './errors'
export {
  type ConfigureHermesRuntimeOptions,
  configureHermesRuntime,
  getHermesRuntime,
  HermesContainerRuntime,
  type HermesContainerRuntimeConfig,
  prepareHermesContext,
} from './hermes-container-runtime'
export {
  HostProcessAgentRuntime,
  type HostProcessAgentRuntimeDeps,
} from './host-process-agent-runtime'
export {
  AgentRuntimeRegistry,
  getAgentRuntimeRegistry,
  resetAgentRuntimeRegistry,
} from './registry'
export type {
  ExecSpec,
  Platform,
  RuntimeAction,
  RuntimeCapability,
  RuntimeDescriptor,
  RuntimeState,
  RuntimeStatusSnapshot,
  StateListener,
  Unsubscribe,
} from './types'
