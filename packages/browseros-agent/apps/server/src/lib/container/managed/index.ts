/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export {
  ContainerNotReadyError,
  PathOutsideMountsError,
  ResetNotSupportedError,
} from './errors'
export {
  ManagedContainer,
  type ManagedContainerDeps,
  type StateListener,
  type Unsubscribe,
} from './managed-container'
export type {
  ContainerDescriptor,
  ContainerState,
  ContainerStatusSnapshot,
  ExecResult,
  ExecSpec,
  MountRoot,
  Platform,
  ResetLevel,
  ResetOptions,
} from './types'
export { TRANSIENT_STATES } from './types'
