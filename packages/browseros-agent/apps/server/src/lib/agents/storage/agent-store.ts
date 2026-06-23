/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentAdapter, AgentDefinition } from '../agent-types'

export interface CreateAgentInput {
  name: string
  adapter: AgentAdapter
  modelId?: string
  reasoningEffort?: string
}

export interface AgentStore {
  list(): Promise<AgentDefinition[]>
  get(id: string): Promise<AgentDefinition | null>
  create(input: CreateAgentInput): Promise<AgentDefinition>
  upsertExisting(input: {
    id: string
    name: string
    adapter: AgentAdapter
    modelId?: string
    reasoningEffort?: string
  }): Promise<AgentDefinition>
  update(
    id: string,
    patch: Partial<Pick<AgentDefinition, 'name' | 'pinned'>>,
  ): Promise<AgentDefinition | null>
  delete(id: string): Promise<boolean>
}
