/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import { desc, eq } from 'drizzle-orm'
import { type BrowserOsDatabase, getDb } from '../db'
import { type AgentDefinitionRow, agentDefinitions } from '../db/schema'
import { logger } from '../logger'
import {
  resolveDefaultModelId,
  resolveDefaultReasoningEffort,
} from './agent-catalog'
import type { AgentStore, CreateAgentInput } from './agent-store'
import type { AgentDefinition } from './agent-types'

/** Persists BrowserOS-owned harness agent definitions in the process SQLite database. */
export class DbAgentStore implements AgentStore {
  private readonly db: BrowserOsDatabase
  private writeQueue: Promise<unknown> = Promise.resolve()

  constructor(options: { db?: BrowserOsDatabase } = {}) {
    this.db = options.db ?? getDb()
  }

  async list(): Promise<AgentDefinition[]> {
    const rows = this.db
      .select()
      .from(agentDefinitions)
      .orderBy(desc(agentDefinitions.updatedAt))
      .all()
    const agents = rows.map(toAgentDefinition)
    logger.debug('Agent harness store listed agents', {
      count: agents.length,
      store: 'sqlite',
    })
    return agents
  }

  async get(id: string): Promise<AgentDefinition | null> {
    const row =
      this.db
        .select()
        .from(agentDefinitions)
        .where(eq(agentDefinitions.id, id))
        .get() ?? null
    return row ? toAgentDefinition(row) : null
  }

  async create(input: CreateAgentInput): Promise<AgentDefinition> {
    return this.withWriteLock(async () => {
      const now = Date.now()
      const id =
        input.adapter === 'openclaw' ? `oc-${randomUUID()}` : randomUUID()
      const row: AgentDefinitionRow = {
        id,
        name: input.name.trim(),
        adapter: input.adapter,
        modelId: input.modelId ?? resolveDefaultModelId(input.adapter),
        reasoningEffort:
          input.reasoningEffort ?? resolveDefaultReasoningEffort(input.adapter),
        permissionMode: 'approve-all',
        sessionKey: `agent:${id}:main`,
        pinned: false,
        adapterConfigJson: serializeAdapterConfig(input),
        createdAt: now,
        updatedAt: now,
      }
      this.db.insert(agentDefinitions).values(row).run()
      const agent = toAgentDefinition(row)
      logger.info('Agent harness store created agent', {
        agentId: agent.id,
        name: agent.name,
        adapter: agent.adapter,
        modelId: agent.modelId,
        reasoningEffort: agent.reasoningEffort,
        sessionKey: agent.sessionKey,
        store: 'sqlite',
      })
      return agent
    })
  }

  /** Backfills a harness record for gateway-side OpenClaw agents discovered during reconciliation. */
  async upsertExisting(input: {
    id: string
    name: string
    adapter: AgentDefinition['adapter']
    modelId?: string
    reasoningEffort?: string
  }): Promise<AgentDefinition> {
    return this.withWriteLock(async () => {
      const existing = await this.get(input.id)
      if (existing) return existing

      const now = Date.now()
      const row: AgentDefinitionRow = {
        id: input.id,
        name: input.name.trim(),
        adapter: input.adapter,
        modelId: input.modelId ?? resolveDefaultModelId(input.adapter),
        reasoningEffort:
          input.reasoningEffort ?? resolveDefaultReasoningEffort(input.adapter),
        permissionMode: 'approve-all',
        sessionKey: `agent:${input.id}:main`,
        pinned: false,
        adapterConfigJson: null,
        createdAt: now,
        updatedAt: now,
      }
      this.db.insert(agentDefinitions).values(row).run()
      const agent = toAgentDefinition(row)
      logger.info('Agent harness store backfilled agent', {
        agentId: agent.id,
        name: agent.name,
        adapter: agent.adapter,
        sessionKey: agent.sessionKey,
        store: 'sqlite',
      })
      return agent
    })
  }

  async update(
    id: string,
    patch: Partial<Pick<AgentDefinition, 'name' | 'pinned'>>,
  ): Promise<AgentDefinition | null> {
    return this.withWriteLock(async () => {
      const current = await this.get(id)
      if (!current) return null

      const values = {
        ...(patch.name !== undefined ? { name: patch.name.trim() } : {}),
        ...(patch.pinned !== undefined ? { pinned: patch.pinned } : {}),
        updatedAt: Date.now(),
      }
      this.db
        .update(agentDefinitions)
        .set(values)
        .where(eq(agentDefinitions.id, id))
        .run()
      return this.get(id)
    })
  }

  async delete(id: string): Promise<boolean> {
    return this.withWriteLock(async () => {
      const existing = await this.get(id)
      if (!existing) return false
      this.db.delete(agentDefinitions).where(eq(agentDefinitions.id, id)).run()
      logger.info('Agent harness store deleted agent', {
        agentId: id,
        store: 'sqlite',
      })
      return true
    })
  }

  private withWriteLock<T>(fn: () => Promise<T>): Promise<T> {
    const result = this.writeQueue.then(fn, fn)
    this.writeQueue = result.then(
      () => undefined,
      () => undefined,
    )
    return result
  }
}

function toAgentDefinition(row: AgentDefinitionRow): AgentDefinition {
  let customCommand: string | undefined
  let customArgs: string[] | undefined
  let customLabel: string | undefined
  if (row.adapterConfigJson) {
    try {
      const parsed = JSON.parse(row.adapterConfigJson)
      if (typeof parsed.customCommand === 'string')
        customCommand = parsed.customCommand
      if (Array.isArray(parsed.customArgs)) customArgs = parsed.customArgs
      if (typeof parsed.customLabel === 'string')
        customLabel = parsed.customLabel
    } catch {}
  }
  return {
    id: row.id,
    name: row.name,
    adapter: row.adapter,
    modelId: row.modelId,
    reasoningEffort: row.reasoningEffort,
    permissionMode: row.permissionMode,
    sessionKey: row.sessionKey,
    pinned: row.pinned,
    ...(customCommand !== undefined ? { customCommand } : {}),
    ...(customArgs !== undefined ? { customArgs } : {}),
    ...(customLabel !== undefined ? { customLabel } : {}),
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

function serializeAdapterConfig(input: CreateAgentInput): string | null {
  const config = {
    ...(input.providerType !== undefined
      ? { providerType: input.providerType }
      : {}),
    ...(input.providerName !== undefined
      ? { providerName: input.providerName }
      : {}),
    ...(input.baseUrl !== undefined ? { baseUrl: input.baseUrl } : {}),
    ...(input.apiKey !== undefined ? { apiKey: input.apiKey } : {}),
    ...(input.supportsImages !== undefined
      ? { supportsImages: input.supportsImages }
      : {}),
    ...(input.customCommand !== undefined
      ? { customCommand: input.customCommand }
      : {}),
    ...(input.customArgs !== undefined ? { customArgs: input.customArgs } : {}),
    ...(input.customLabel !== undefined
      ? { customLabel: input.customLabel }
      : {}),
  }
  return Object.keys(config).length > 0 ? JSON.stringify(config) : null
}
