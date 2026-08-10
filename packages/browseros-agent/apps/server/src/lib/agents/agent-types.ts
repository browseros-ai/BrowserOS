/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AcpAgentType } from '@browseros/shared/schemas/agent'

export type { AcpAgentType }

export interface AcpAgentDefinition {
  id: string
  name: string
  type: AcpAgentType
  modelId?: string
  reasoningEffort?: string
  workingDirectory?: string
  createdAt: number
  updatedAt: number
}
