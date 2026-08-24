import { randomUUID } from 'node:crypto'
import { CustomAcpAgentConfigSchema } from '@browseros/shared/schemas/agent'
import { desc, eq } from 'drizzle-orm'
import { type BrowserOsDatabase, getDb } from '../../db'
import { type AcpAgentRow, acpAgents } from '../../db/schema'
import { logger } from '../../logger'
import type {
  AcpAgentDefinition,
  AcpAgentType,
  CustomAcpAgentConfig,
} from '../agent-types'

export interface CreateAcpAgentInput {
  name: string
  type: AcpAgentType
  modelId?: string
  reasoningEffort?: string
  workingDirectory?: string
  customConfig?: CustomAcpAgentConfig
}

export interface UpdateAcpAgentInput {
  name?: string
  modelId?: string | null
  reasoningEffort?: string | null
  workingDirectory?: string | null
  customConfig?: CustomAcpAgentConfig
}

export interface AcpAgentStore {
  list(): Promise<AcpAgentDefinition[]>
  get(id: string): Promise<AcpAgentDefinition | null>
  create(input: CreateAcpAgentInput): Promise<AcpAgentDefinition>
  update(
    id: string,
    input: UpdateAcpAgentInput,
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
        customConfig: input.customConfig
          ? JSON.stringify(input.customConfig)
          : null,
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
    input: UpdateAcpAgentInput,
  ): Promise<AcpAgentDefinition | null> {
    return this.withWriteLock(async () => {
      const existing =
        this.db.select().from(acpAgents).where(eq(acpAgents.id, id)).get() ??
        null
      if (!existing) return null

      const next: AcpAgentRow = {
        ...existing,
        name: input.name === undefined ? existing.name : input.name.trim(),
        modelId:
          input.modelId === undefined
            ? existing.modelId
            : optionalText(input.modelId ?? undefined),
        reasoningEffort:
          input.reasoningEffort === undefined
            ? existing.reasoningEffort
            : optionalText(input.reasoningEffort ?? undefined),
        workingDirectory:
          input.workingDirectory === undefined
            ? existing.workingDirectory
            : optionalText(input.workingDirectory ?? undefined),
        customConfig:
          input.customConfig === undefined
            ? existing.customConfig
            : JSON.stringify(input.customConfig),
        updatedAt: Date.now(),
      }
      this.db.update(acpAgents).set(next).where(eq(acpAgents.id, id)).run()
      logger.info('ACP agent updated', { agentId: id })
      return toAcpAgentDefinition(next)
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
    customConfig: parseCustomConfig(row.customConfig, row.id),
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

function parseCustomConfig(
  raw: string | null,
  agentId: string,
): CustomAcpAgentConfig | undefined {
  if (!raw) return undefined
  let json: unknown
  try {
    json = JSON.parse(raw)
  } catch {
    logger.warn('Ignoring unparseable custom agent config', { agentId })
    return undefined
  }
  const parsed = CustomAcpAgentConfigSchema.safeParse(json)
  if (!parsed.success) {
    logger.warn('Ignoring malformed custom agent config', { agentId })
    return undefined
  }
  return parsed.data
}

function optionalText(value: string | undefined): string | null {
  return value?.trim() || null
}
