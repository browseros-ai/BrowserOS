/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  AcpAgentType,
  CustomAcpAgentConfig,
} from '@browseros/shared/schemas/agent'

export type { AcpAgentType, CustomAcpAgentConfig }

export interface AcpAgentDefinition {
  id: string
  name: string
  type: AcpAgentType
  modelId?: string
  reasoningEffort?: string
  workingDirectory?: string
  /** Present only for `type: 'custom'` agents. */
  customConfig?: CustomAcpAgentConfig
  createdAt: number
  updatedAt: number
}
