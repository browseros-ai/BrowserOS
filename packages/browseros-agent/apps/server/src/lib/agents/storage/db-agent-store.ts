/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import { desc, eq } from 'drizzle-orm'
import { type BrowserOsDatabase, getDb } from '../../db'
import { type AgentDefinitionRow, agentDefinitions } from '../../db/schema'
import { logger } from '../../logger'
import {
  isAgentAdapter,
  resolveDefaultModelId,
  resolveDefaultReasoningEffort,
} from '../adapters/catalog'
import type { AgentDefinition } from '../agent-types'
import type { AgentStore, CreateAgentInput } from './agent-store'

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
    const agents = rows.flatMap((row) => {
      const agent = toAgentDefinition(row)
      return agent ? [agent] : []
    })
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
      const id = randomUUID()
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
        adapterConfigJson: null,
        createdAt: now,
        updatedAt: now,
      }
      this.db.insert(agentDefinitions).values(row).run()
      const agent = toAgentDefinition(row)
      if (!agent)
        throw new Error(`created unsupported agent adapter ${row.adapter}`)
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

  /** Inserts an existing agent id when importing records from another store. */
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
      if (!agent) {
        throw new Error(`backfilled unsupported agent adapter ${row.adapter}`)
      }
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

function toAgentDefinition(row: AgentDefinitionRow): AgentDefinition | null {
  const adapter = row.adapter as unknown
  if (!isAgentAdapter(adapter)) {
    logger.warn('Agent harness store ignored unsupported adapter row', {
      agentId: row.id,
      adapter,
      store: 'sqlite',
    })
    return null
  }
  return {
    id: row.id,
    name: row.name,
    adapter,
    modelId: row.modelId,
    reasoningEffort: row.reasoningEffort,
    permissionMode: row.permissionMode,
    sessionKey: row.sessionKey,
    pinned: row.pinned,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}
