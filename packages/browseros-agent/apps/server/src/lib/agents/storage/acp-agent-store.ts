import { randomUUID } from 'node:crypto'
import { CustomAcpAgentConfigSchema } from '@browseros/shared/schemas/agent'
import { and, desc, eq } from 'drizzle-orm'
import { type BrowserOsDatabase, getDb } from '../../db'
import { type ProviderRow, providers } from '../../db/schema'
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

/**
 * ACP agents are rows in the unified providers table, distinguished by kind.
 *
 * The store keeps its own shape rather than folding into the provider store:
 * agents are created with a generated id and updated field by field, where
 * providers are upserted under an id the client already holds. Both are
 * legitimate ways to reach the same table, and the shipped /agents contract
 * depends on this one.
 */
const isAgent = eq(providers.kind, 'acp')

export class DbAcpAgentStore implements AcpAgentStore {
  private readonly db: BrowserOsDatabase
  private writeQueue: Promise<unknown> = Promise.resolve()

  constructor(options: { db?: BrowserOsDatabase } = {}) {
    this.db = options.db ?? getDb()
  }

  async list(): Promise<AcpAgentDefinition[]> {
    return this.db
      .select()
      .from(providers)
      .where(isAgent)
      .orderBy(desc(providers.updatedAt))
      .all()
      .map(toAcpAgentDefinition)
  }

  async get(id: string): Promise<AcpAgentDefinition | null> {
    const row =
      this.db
        .select()
        .from(providers)
        .where(and(isAgent, eq(providers.id, id)))
        .get() ?? null
    return row ? toAcpAgentDefinition(row) : null
  }

  async create(input: CreateAcpAgentInput): Promise<AcpAgentDefinition> {
    return this.withWriteLock(async () => {
      const now = Date.now()
      const row = {
        id: randomUUID(),
        kind: 'acp' as const,
        profileId: null,
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
      // returning(), not the object built above: the unified table fills the
      // columns only LLM providers use, so the row that lands is wider than
      // what was inserted.
      const saved = this.db.insert(providers).values(row).returning().get()
      const agent = toAcpAgentDefinition(saved)
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
        this.db
          .select()
          .from(providers)
          .where(and(isAgent, eq(providers.id, id)))
          .get() ?? null
      if (!existing) return null

      const next: ProviderRow = {
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
      this.db
        .update(providers)
        .set(next)
        .where(and(isAgent, eq(providers.id, id)))
        .run()
      logger.info('ACP agent updated', { agentId: id })
      return toAcpAgentDefinition(next)
    })
  }

  async delete(id: string): Promise<boolean> {
    return this.withWriteLock(async () => {
      if (!(await this.get(id))) return false
      this.db
        .delete(providers)
        .where(and(isAgent, eq(providers.id, id)))
        .run()
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

function toAcpAgentDefinition(row: ProviderRow): AcpAgentDefinition {
  return {
    id: row.id,
    name: row.name,
    type: row.type as AcpAgentType,
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
