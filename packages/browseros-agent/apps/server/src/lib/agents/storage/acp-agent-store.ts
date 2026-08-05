import { randomUUID } from 'node:crypto'
import { desc, eq } from 'drizzle-orm'
import { type BrowserOsDatabase, getDb } from '../../db'
import { type AcpAgentRow, acpAgents } from '../../db/schema'
import { logger } from '../../logger'
import type { AcpAgentDefinition, AcpAgentType } from '../agent-types'

export interface CreateAcpAgentInput {
  name: string
  type: AcpAgentType
  modelId?: string
  reasoningEffort?: string
  workingDirectory?: string
}

export interface AcpAgentStore {
  list(): Promise<AcpAgentDefinition[]>
  get(id: string): Promise<AcpAgentDefinition | null>
  create(input: CreateAcpAgentInput): Promise<AcpAgentDefinition>
  update(
    id: string,
    patch: Partial<Pick<AcpAgentDefinition, 'name' | 'pinned'>>,
  ): Promise<AcpAgentDefinition | null>
  delete(id: string): Promise<boolean>
}

export class DbAcpAgentStore implements AcpAgentStore {
  private readonly db: BrowserOsDatabase
  private writeQueue: Promise<unknown> = Promise.resolve()

  constructor(options: { db?: BrowserOsDatabase } = {}) {
    this.db = options.db ?? getDb()
  }

  async list(): Promise<AcpAgentDefinition[]> {
    return this.db
      .select()
      .from(acpAgents)
      .orderBy(desc(acpAgents.updatedAt))
      .all()
      .map(toAcpAgentDefinition)
  }

  async get(id: string): Promise<AcpAgentDefinition | null> {
    const row =
      this.db.select().from(acpAgents).where(eq(acpAgents.id, id)).get() ?? null
    return row ? toAcpAgentDefinition(row) : null
  }

  async create(input: CreateAcpAgentInput): Promise<AcpAgentDefinition> {
    return this.withWriteLock(async () => {
      const now = Date.now()
      const row: AcpAgentRow = {
        id: randomUUID(),
        name: input.name.trim(),
        type: input.type,
        modelId: optionalText(input.modelId),
        reasoningEffort: optionalText(input.reasoningEffort),
        workingDirectory: optionalText(input.workingDirectory),
        pinned: false,
        createdAt: now,
        updatedAt: now,
      }
      this.db.insert(acpAgents).values(row).run()
      const agent = toAcpAgentDefinition(row)
      logger.info('ACP agent created', {
        agentId: agent.id,
        type: agent.type,
      })
      return agent
    })
  }

  async update(
    id: string,
    patch: Partial<Pick<AcpAgentDefinition, 'name' | 'pinned'>>,
  ): Promise<AcpAgentDefinition | null> {
    return this.withWriteLock(async () => {
      if (!(await this.get(id))) return null
      this.db
        .update(acpAgents)
        .set({
          ...(patch.name !== undefined ? { name: patch.name.trim() } : {}),
          ...(patch.pinned !== undefined ? { pinned: patch.pinned } : {}),
          updatedAt: Date.now(),
        })
        .where(eq(acpAgents.id, id))
        .run()
      return this.get(id)
    })
  }

  async delete(id: string): Promise<boolean> {
    return this.withWriteLock(async () => {
      if (!(await this.get(id))) return false
      this.db.delete(acpAgents).where(eq(acpAgents.id, id)).run()
      logger.info('ACP agent deleted', { agentId: id })
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

export function deriveAcpSessionKey(
  agentId: string,
  conversationId: string,
): string {
  return `acp:${agentId}:${conversationId}`
}

function toAcpAgentDefinition(row: AcpAgentRow): AcpAgentDefinition {
  return {
    id: row.id,
    name: row.name,
    type: row.type,
    modelId: row.modelId ?? undefined,
    reasoningEffort: row.reasoningEffort ?? undefined,
    workingDirectory: row.workingDirectory ?? undefined,
    pinned: row.pinned,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

function optionalText(value: string | undefined): string | null {
  return value?.trim() || null
}
