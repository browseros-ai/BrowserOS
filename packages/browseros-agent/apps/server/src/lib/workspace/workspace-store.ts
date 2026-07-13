/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createHash } from 'node:crypto'
import { promises as fs } from 'node:fs'
import { basename, dirname, isAbsolute, join, relative, resolve } from 'node:path'
import {
  and,
  asc,
  desc,
  eq,
} from 'drizzle-orm'
import type { BrowserOsDatabase } from '../db'
import { getBrowserosDir } from '../browseros-dir'
import { getDb } from '../db'
import {
  researchEvents,
  researchPlanSteps,
  researchSessions,
  workspaceAssets,
  workspaceCollections,
  workspaceDatabases,
  workspaceFields,
  workspaceRecords,
  workspaceSources,
  type ResearchEventRow,
  type ResearchPlanStepRow,
  type ResearchSessionRow,
  type WorkspaceAssetRow,
  type WorkspaceCollectionRow,
  type WorkspaceDatabaseRow,
  type WorkspaceFieldRow,
  type WorkspaceRecordRow,
  type WorkspaceSourceRow,
} from '../db/schema'

export type WorkspaceFieldType =
  | 'text'
  | 'number'
  | 'currency'
  | 'date'
  | 'boolean'
  | 'select'
  | 'multi-select'
  | 'url'
  | 'image'
  | 'location'
  | 'source'
  | 'asset'

export type ResearchSessionStatus =
  | 'draft'
  | 'planning'
  | 'awaiting_plan_approval'
  | 'running'
  | 'awaiting_user'
  | 'paused'
  | 'verifying'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type ResearchPlanStepStatus =
  | 'pending'
  | 'running'
  | 'completed'
  | 'blocked'
  | 'skipped'
  | 'failed'

export type ResearchToolCategory =
  | 'browser'
  | 'extraction'
  | 'database'
  | 'analysis'
  | 'report'

export type ResearchEventKind =
  | 'activity'
  | 'reasoning-summary'
  | 'decision'
  | 'tool-call'
  | 'error'
  | 'checkpoint'

export interface WorkspaceStoreOptions {
  db?: BrowserOsDatabase
  assetsDir?: string
}

export interface WorkspaceDatabaseWithFields extends WorkspaceDatabaseRow {
  fields: WorkspaceFieldRow[]
}

export interface WorkspaceRecord extends Omit<WorkspaceRecordRow, 'dataJson'> {
  data: Record<string, unknown>
}

export interface ResearchSessionWithActivity extends ResearchSessionRow {
  plan: ResearchPlanStepRow[]
  events: ResearchEventRow[]
  recap: SessionRecap | null
}

export interface SessionRecap {
  sessionId: string
  goal: string
  status: ResearchSessionStatus
  plan: {
    total: number
    completed: number
    blocked: number
    remaining: number
  }
  activityCount: number
  nextActions: string[]
  generatedAt: number
}

export interface SessionSuggestion {
  sessionId: string
  message: string
  basedOn: string
  generatedAt: number
}

export interface CreateSessionInput {
  goal: string
  conversationId?: string
  browserProfileId?: string
  collectionId?: string
  databaseId?: string
  status?: ResearchSessionStatus
  plan?: Array<{
    title: string
    description?: string
    toolCategory: ResearchToolCategory
    requiresApproval?: boolean
    expectedOutput?: string
  }>
}

export interface CreateAssetInput {
  filename: string
  mimeType: string
  data: Uint8Array
  sessionId?: string
  recordId?: string
  sourceId?: string
  width?: number
  height?: number
}

export const WORKSPACE_ASSET_DIR_MODE = 0o700
export const WORKSPACE_ASSET_FILE_MODE = 0o600
export const MAX_WORKSPACE_ASSET_BYTES = 25 * 1024 * 1024

export class WorkspaceStore {
  private readonly db: BrowserOsDatabase
  private readonly assetsDir: string

  constructor(options: WorkspaceStoreOptions = {}) {
    this.db = options.db ?? getDb()
    this.assetsDir = resolve(
      options.assetsDir ?? join(getBrowserosDir(), 'workspace-assets'),
    )
  }

  async listCollections(): Promise<WorkspaceCollectionRow[]> {
    return this.db
      .select()
      .from(workspaceCollections)
      .orderBy(desc(workspaceCollections.updatedAt))
      .all()
  }

  async createCollection(input: {
    name: string
    description?: string
  }): Promise<WorkspaceCollectionRow> {
    const name = requireText(input.name, 'Collection name')
    const now = Date.now()
    const row: WorkspaceCollectionRow = {
      id: crypto.randomUUID(),
      name,
      description: optionalText(input.description),
      createdAt: now,
      updatedAt: now,
    }
    this.db.insert(workspaceCollections).values(row).run()
    return row
  }

  async updateCollection(
    id: string,
    patch: { name?: string; description?: string | null },
  ): Promise<WorkspaceCollectionRow | null> {
    const values: Partial<WorkspaceCollectionRow> = {
      updatedAt: Date.now(),
    }
    if (patch.name !== undefined) values.name = requireText(patch.name, 'Collection name')
    if (patch.description !== undefined) {
      values.description = optionalText(patch.description)
    }
    this.db
      .update(workspaceCollections)
      .set(values)
      .where(eq(workspaceCollections.id, id))
      .run()
    return this.db
      .select()
      .from(workspaceCollections)
      .where(eq(workspaceCollections.id, id))
      .get() ?? null
  }

  async deleteCollection(id: string): Promise<boolean> {
    const existing = this.db
      .select({ id: workspaceCollections.id })
      .from(workspaceCollections)
      .where(eq(workspaceCollections.id, id))
      .get()
    if (!existing) return false
    this.db.delete(workspaceCollections).where(eq(workspaceCollections.id, id)).run()
    return true
  }

  async listDatabases(collectionId?: string): Promise<WorkspaceDatabaseRow[]> {
    const query = this.db.select().from(workspaceDatabases)
    return collectionId
      ? query
          .where(eq(workspaceDatabases.collectionId, collectionId))
          .orderBy(desc(workspaceDatabases.updatedAt))
          .all()
      : query.orderBy(desc(workspaceDatabases.updatedAt)).all()
  }

  async getDatabase(id: string): Promise<WorkspaceDatabaseWithFields | null> {
    const database = this.db
      .select()
      .from(workspaceDatabases)
      .where(eq(workspaceDatabases.id, id))
      .get()
    if (!database) return null
    return {
      ...database,
      fields: this.db
        .select()
        .from(workspaceFields)
        .where(eq(workspaceFields.databaseId, id))
        .orderBy(asc(workspaceFields.position), asc(workspaceFields.createdAt))
        .all(),
    }
  }

  async createDatabase(input: {
    name: string
    description?: string
    collectionId?: string
  }): Promise<WorkspaceDatabaseRow> {
    const name = requireText(input.name, 'Database name')
    const now = Date.now()
    const row: WorkspaceDatabaseRow = {
      id: crypto.randomUUID(),
      collectionId: input.collectionId ?? null,
      name,
      description: optionalText(input.description),
      createdAt: now,
      updatedAt: now,
    }
    this.db.insert(workspaceDatabases).values(row).run()
    return row
  }

  async updateDatabase(
    id: string,
    patch: {
      name?: string
      description?: string | null
      collectionId?: string | null
    },
  ): Promise<WorkspaceDatabaseRow | null> {
    const values: Partial<WorkspaceDatabaseRow> = {
      updatedAt: Date.now(),
    }
    if (patch.name !== undefined) values.name = requireText(patch.name, 'Database name')
    if (patch.description !== undefined) {
      values.description = optionalText(patch.description)
    }
    if (patch.collectionId !== undefined) values.collectionId = patch.collectionId
    this.db
      .update(workspaceDatabases)
      .set(values)
      .where(eq(workspaceDatabases.id, id))
      .run()
    return this.db
      .select()
      .from(workspaceDatabases)
      .where(eq(workspaceDatabases.id, id))
      .get() ?? null
  }

  async deleteDatabase(id: string): Promise<boolean> {
    const existing = this.db
      .select({ id: workspaceDatabases.id })
      .from(workspaceDatabases)
      .where(eq(workspaceDatabases.id, id))
      .get()
    if (!existing) return false
    const records = this.db
      .select({ id: workspaceRecords.id })
      .from(workspaceRecords)
      .where(eq(workspaceRecords.databaseId, id))
      .all()
    for (const record of records) {
      await this.removeAssetFilesForRecord(record.id)
    }
    this.db.delete(workspaceDatabases).where(eq(workspaceDatabases.id, id)).run()
    return true
  }

  async createField(input: {
    databaseId: string
    name: string
    key?: string
    type: WorkspaceFieldType
    position?: number
    required?: boolean
    configuration?: Record<string, unknown>
  }): Promise<WorkspaceFieldRow> {
    const name = requireText(input.name, 'Field name')
    const now = Date.now()
    const row: WorkspaceFieldRow = {
      id: crypto.randomUUID(),
      databaseId: input.databaseId,
      name,
      key: slugify(input.key || name),
      type: input.type,
      position: input.position ?? 0,
      required: input.required ?? false,
      configurationJson: input.configuration
        ? stringifyJson(input.configuration)
        : null,
      createdAt: now,
      updatedAt: now,
    }
    this.db.insert(workspaceFields).values(row).run()
    return row
  }

  async updateField(
    id: string,
    patch: {
      name?: string
      key?: string
      type?: WorkspaceFieldType
      position?: number
      required?: boolean
      configuration?: Record<string, unknown> | null
    },
  ): Promise<WorkspaceFieldRow | null> {
    const values: Partial<WorkspaceFieldRow> = {
      updatedAt: Date.now(),
    }
    if (patch.name !== undefined) values.name = requireText(patch.name, 'Field name')
    if (patch.key !== undefined) values.key = slugify(patch.key)
    if (patch.type !== undefined) values.type = patch.type
    if (patch.position !== undefined) values.position = patch.position
    if (patch.required !== undefined) values.required = patch.required
    if (patch.configuration !== undefined) {
      values.configurationJson = patch.configuration
        ? stringifyJson(patch.configuration)
        : null
    }
    this.db.update(workspaceFields).set(values).where(eq(workspaceFields.id, id)).run()
    return this.db
      .select()
      .from(workspaceFields)
      .where(eq(workspaceFields.id, id))
      .get() ?? null
  }

  async deleteField(id: string): Promise<boolean> {
    const existing = this.db
      .select({ id: workspaceFields.id })
      .from(workspaceFields)
      .where(eq(workspaceFields.id, id))
      .get()
    if (!existing) return false
    this.db.delete(workspaceFields).where(eq(workspaceFields.id, id)).run()
    return true
  }

  async listRecords(
    databaseId: string,
    options: { limit?: number; offset?: number } = {},
  ): Promise<WorkspaceRecord[]> {
    const limit = clampInteger(options.limit ?? 100, 1, 1000)
    const offset = clampInteger(options.offset ?? 0, 0, Number.MAX_SAFE_INTEGER)
    const rows = this.db
      .select()
      .from(workspaceRecords)
      .where(eq(workspaceRecords.databaseId, databaseId))
      .orderBy(desc(workspaceRecords.updatedAt))
      .limit(limit)
      .offset(offset)
      .all()
    return rows.map(toWorkspaceRecord)
  }

  async createRecord(input: {
    databaseId: string
    title?: string
    data: Record<string, unknown>
    sessionId?: string
    sourceId?: string
  }): Promise<WorkspaceRecord> {
    const now = Date.now()
    const row: WorkspaceRecordRow = {
      id: crypto.randomUUID(),
      databaseId: input.databaseId,
      sessionId: input.sessionId ?? null,
      sourceId: input.sourceId ?? null,
      title: optionalText(input.title),
      dataJson: stringifyJson(input.data),
      createdAt: now,
      updatedAt: now,
    }
    this.db.insert(workspaceRecords).values(row).run()
    return toWorkspaceRecord(row)
  }

  async updateRecord(
    id: string,
    patch: {
      title?: string | null
      data?: Record<string, unknown>
      sessionId?: string | null
      sourceId?: string | null
    },
  ): Promise<WorkspaceRecord | null> {
    const values: Partial<WorkspaceRecordRow> = {
      updatedAt: Date.now(),
    }
    if (patch.title !== undefined) values.title = optionalText(patch.title)
    if (patch.data !== undefined) values.dataJson = stringifyJson(patch.data)
    if (patch.sessionId !== undefined) values.sessionId = patch.sessionId
    if (patch.sourceId !== undefined) values.sourceId = patch.sourceId
    this.db.update(workspaceRecords).set(values).where(eq(workspaceRecords.id, id)).run()
    const row = this.db
      .select()
      .from(workspaceRecords)
      .where(eq(workspaceRecords.id, id))
      .get()
    return row ? toWorkspaceRecord(row) : null
  }

  async getRecord(id: string): Promise<WorkspaceRecord | null> {
    const row = this.db
      .select()
      .from(workspaceRecords)
      .where(eq(workspaceRecords.id, id))
      .get()
    return row ? toWorkspaceRecord(row) : null
  }

  async deleteRecord(id: string): Promise<boolean> {
    const existing = this.db
      .select({ id: workspaceRecords.id })
      .from(workspaceRecords)
      .where(eq(workspaceRecords.id, id))
      .get()
    if (!existing) return false
    await this.removeAssetFilesForRecord(id)
    this.db.delete(workspaceRecords).where(eq(workspaceRecords.id, id)).run()
    return true
  }

  async createSource(input: {
    url: string
    title?: string
    excerpt?: string
    contentHash?: string
    snapshotPath?: string
    sessionId?: string
    accessedAt?: number
  }): Promise<WorkspaceSourceRow> {
    const url = requireText(input.url, 'Source URL')
    const now = Date.now()
    const row: WorkspaceSourceRow = {
      id: crypto.randomUUID(),
      sessionId: input.sessionId ?? null,
      url,
      title: optionalText(input.title),
      excerpt: optionalText(input.excerpt),
      contentHash: optionalText(input.contentHash),
      snapshotPath: optionalText(input.snapshotPath),
      accessedAt: input.accessedAt ?? now,
      createdAt: now,
    }
    this.db.insert(workspaceSources).values(row).run()
    return row
  }

  async getSource(id: string): Promise<WorkspaceSourceRow | null> {
    return this.db
      .select()
      .from(workspaceSources)
      .where(eq(workspaceSources.id, id))
      .get() ?? null
  }

  async listSources(sessionId?: string): Promise<WorkspaceSourceRow[]> {
    const query = this.db.select().from(workspaceSources)
    return sessionId
      ? query
          .where(eq(workspaceSources.sessionId, sessionId))
          .orderBy(desc(workspaceSources.accessedAt))
          .all()
      : query.orderBy(desc(workspaceSources.accessedAt)).all()
  }

  async listAssets(filters: {
    sessionId?: string
    recordId?: string
    sourceId?: string
  } = {}): Promise<WorkspaceAssetRow[]> {
    const conditions = [
      filters.sessionId
        ? eq(workspaceAssets.sessionId, filters.sessionId)
        : undefined,
      filters.recordId ? eq(workspaceAssets.recordId, filters.recordId) : undefined,
      filters.sourceId ? eq(workspaceAssets.sourceId, filters.sourceId) : undefined,
    ].filter(Boolean)
    const query = this.db.select().from(workspaceAssets)
    return conditions.length > 0
      ? query.where(and(...conditions)).orderBy(desc(workspaceAssets.createdAt)).all()
      : query.orderBy(desc(workspaceAssets.createdAt)).all()
  }

  async createAsset(input: CreateAssetInput): Promise<WorkspaceAssetRow> {
    if (input.data.byteLength > MAX_WORKSPACE_ASSET_BYTES) {
      throw new Error(`Asset exceeds ${MAX_WORKSPACE_ASSET_BYTES} bytes`)
    }
    const filename = sanitizeFilename(input.filename)
    const mimeType = requireText(input.mimeType, 'Asset MIME type')
    const id = crypto.randomUUID()
    const directory = join(this.assetsDir, id)
    const filePath = join(directory, filename)
    const storageKey = `${id}/${filename}`
    const contentHash = createHash('sha256').update(input.data).digest('hex')
    const now = Date.now()

    await fs.mkdir(directory, { recursive: true, mode: WORKSPACE_ASSET_DIR_MODE })
    await fs.chmod(this.assetsDir, WORKSPACE_ASSET_DIR_MODE).catch(() => {})
    await fs.chmod(directory, WORKSPACE_ASSET_DIR_MODE).catch(() => {})
    try {
      await fs.writeFile(filePath, input.data, {
        flag: 'wx',
        mode: WORKSPACE_ASSET_FILE_MODE,
      })
      await fs.chmod(filePath, WORKSPACE_ASSET_FILE_MODE).catch(() => {})
      const row: WorkspaceAssetRow = {
        id,
        sessionId: input.sessionId ?? null,
        recordId: input.recordId ?? null,
        sourceId: input.sourceId ?? null,
        filename,
        mimeType,
        storageKey,
        byteSize: input.data.byteLength,
        contentHash,
        width: input.width ?? null,
        height: input.height ?? null,
        createdAt: now,
      }
      this.db.insert(workspaceAssets).values(row).run()
      return row
    } catch (error) {
      await fs.rm(directory, { recursive: true, force: true }).catch(() => {})
      throw error
    }
  }

  async getAsset(id: string): Promise<WorkspaceAssetRow | null> {
    return this.db
      .select()
      .from(workspaceAssets)
      .where(eq(workspaceAssets.id, id))
      .get() ?? null
  }

  async readAsset(id: string): Promise<{
    asset: WorkspaceAssetRow
    data: Uint8Array
  } | null> {
    const asset = await this.getAsset(id)
    if (!asset) return null
    const filePath = this.resolveAssetPath(asset.storageKey)
    return { asset, data: await fs.readFile(filePath) }
  }

  async deleteAsset(id: string): Promise<boolean> {
    const asset = await this.getAsset(id)
    if (!asset) return false
    const filePath = this.resolveAssetPath(asset.storageKey)
    await fs.rm(dirname(filePath), { recursive: true, force: true })
    this.db.delete(workspaceAssets).where(eq(workspaceAssets.id, id)).run()
    return true
  }

  async listResearchSessions(limit = 100): Promise<ResearchSessionRow[]> {
    return this.db
      .select()
      .from(researchSessions)
      .orderBy(desc(researchSessions.updatedAt))
      .limit(clampInteger(limit, 1, 1000))
      .all()
  }

  async createResearchSession(input: CreateSessionInput): Promise<ResearchSessionWithActivity> {
    const goal = requireText(input.goal, 'Research goal')
    const now = Date.now()
    const session: ResearchSessionRow = {
      id: crypto.randomUUID(),
      conversationId: optionalText(input.conversationId),
      goal,
      status: input.status ?? 'draft',
      activeStepId: null,
      browserProfileId: optionalText(input.browserProfileId),
      collectionId: input.collectionId ?? null,
      databaseId: input.databaseId ?? null,
      recapJson: null,
      startedAt: now,
      updatedAt: now,
      completedAt: null,
    }
    this.db.transaction((tx) => {
      tx.insert(researchSessions).values(session).run()
      if (input.plan?.length) {
        tx.insert(researchPlanSteps)
          .values(
            input.plan.map((step, index): ResearchPlanStepRow => ({
              id: crypto.randomUUID(),
              sessionId: session.id,
              stepOrder: index,
              title: requireText(step.title, 'Plan step title'),
              description: optionalText(step.description),
              status: 'pending',
              toolCategory: step.toolCategory,
              requiresApproval: step.requiresApproval ?? false,
              expectedOutput: optionalText(step.expectedOutput),
              error: null,
              startedAt: null,
              completedAt: null,
            })),
          )
          .run()
      }
    })
    return (await this.getResearchSession(session.id)) as ResearchSessionWithActivity
  }

  async getResearchSession(id: string): Promise<ResearchSessionWithActivity | null> {
    const session = this.db
      .select()
      .from(researchSessions)
      .where(eq(researchSessions.id, id))
      .get()
    if (!session) return null
    const plan = this.db
      .select()
      .from(researchPlanSteps)
      .where(eq(researchPlanSteps.sessionId, id))
      .orderBy(asc(researchPlanSteps.stepOrder))
      .all()
    const events = this.db
      .select()
      .from(researchEvents)
      .where(eq(researchEvents.sessionId, id))
      .orderBy(asc(researchEvents.createdAt))
      .all()
    return {
      ...session,
      plan,
      events,
      recap: parseJson<SessionRecap>(session.recapJson),
    }
  }

  async getResearchSessionByConversationId(
    conversationId: string,
  ): Promise<ResearchSessionWithActivity | null> {
    const session = this.db
      .select({ id: researchSessions.id })
      .from(researchSessions)
      .where(eq(researchSessions.conversationId, conversationId))
      .orderBy(desc(researchSessions.updatedAt))
      .get()
    return session ? this.getResearchSession(session.id) : null
  }

  async updateResearchSession(
    id: string,
    patch: {
      status?: ResearchSessionStatus
      activeStepId?: string | null
      collectionId?: string | null
      databaseId?: string | null
      browserProfileId?: string | null
    },
  ): Promise<ResearchSessionWithActivity | null> {
    const values: Partial<ResearchSessionRow> = { updatedAt: Date.now() }
    if (patch.status !== undefined) {
      values.status = patch.status
      if (patch.status === 'completed') values.completedAt = Date.now()
    }
    if (patch.activeStepId !== undefined) values.activeStepId = patch.activeStepId
    if (patch.collectionId !== undefined) values.collectionId = patch.collectionId
    if (patch.databaseId !== undefined) values.databaseId = patch.databaseId
    if (patch.browserProfileId !== undefined) {
      values.browserProfileId = patch.browserProfileId
    }
    this.db.update(researchSessions).set(values).where(eq(researchSessions.id, id)).run()
    return this.getResearchSession(id)
  }

  async updatePlanStep(
    id: string,
    patch: {
      status?: ResearchPlanStepStatus
      title?: string
      description?: string | null
      requiresApproval?: boolean
      error?: string | null
    },
  ): Promise<ResearchPlanStepRow | null> {
    const values: Partial<ResearchPlanStepRow> = {}
    if (patch.status !== undefined) {
      values.status = patch.status
      if (patch.status === 'running') values.startedAt = Date.now()
      if (['completed', 'failed', 'skipped'].includes(patch.status)) {
        values.completedAt = Date.now()
      }
    }
    if (patch.title !== undefined) values.title = requireText(patch.title, 'Plan step title')
    if (patch.description !== undefined) values.description = optionalText(patch.description)
    if (patch.requiresApproval !== undefined) values.requiresApproval = patch.requiresApproval
    if (patch.error !== undefined) values.error = optionalText(patch.error)
    this.db.update(researchPlanSteps).set(values).where(eq(researchPlanSteps.id, id)).run()
    return this.db
      .select()
      .from(researchPlanSteps)
      .where(eq(researchPlanSteps.id, id))
      .get() ?? null
  }

  async addResearchEvent(input: {
    sessionId: string
    kind: ResearchEventKind
    title: string
    detail?: string
    payload?: Record<string, unknown>
  }): Promise<ResearchEventRow> {
    const row: ResearchEventRow = {
      id: crypto.randomUUID(),
      sessionId: input.sessionId,
      kind: input.kind,
      title: requireText(input.title, 'Event title'),
      detail: optionalText(input.detail),
      payloadJson: input.payload ? stringifyJson(input.payload) : null,
      createdAt: Date.now(),
    }
    this.db.insert(researchEvents).values(row).run()
    return row
  }

  async buildSessionRecap(id: string): Promise<SessionRecap | null> {
    const session = await this.getResearchSession(id)
    if (!session) return null
    const completed = session.plan.filter((step) => step.status === 'completed').length
    const blocked = session.plan.filter((step) => step.status === 'blocked').length
    const remaining = session.plan.filter((step) =>
      ['pending', 'running', 'blocked'].includes(step.status),
    ).length
    const recap: SessionRecap = {
      sessionId: session.id,
      goal: session.goal,
      status: session.status,
      plan: {
        total: session.plan.length,
        completed,
        blocked,
        remaining,
      },
      activityCount: session.events.length,
      nextActions: session.plan
        .filter((step) => ['pending', 'running', 'blocked'].includes(step.status))
        .slice(0, 5)
        .map((step) => step.title),
      generatedAt: Date.now(),
    }
    this.db
      .update(researchSessions)
      .set({ recapJson: stringifyJson(recap), updatedAt: Date.now() })
      .where(eq(researchSessions.id, id))
      .run()
    return recap
  }

  async buildSessionSuggestion(id: string): Promise<SessionSuggestion | null> {
    const session = await this.getResearchSession(id)
    if (!session) return null
    const recap = session.recap ?? (await this.buildSessionRecap(id))
    const nextAction = recap?.nextActions[0]
    const message = nextAction
      ? `Continue this research by ${lowercaseFirst(nextAction)}. Keep the results source-backed and save the verified findings to the selected database.`
      : `Review the saved results for "${session.goal}" and summarize the strongest findings, gaps, and recommended next action.`
    return {
      sessionId: id,
      message,
      basedOn: nextAction ?? session.goal,
      generatedAt: Date.now(),
    }
  }

  private resolveAssetPath(storageKey: string): string {
    const root = resolve(this.assetsDir)
    const filePath = resolve(root, storageKey)
    const rel = relative(root, filePath)
    if (!rel || rel.startsWith('..') || isAbsolute(rel)) {
      throw new Error('Workspace asset path escapes the asset directory')
    }
    return filePath
  }

  private async removeAssetFilesForRecord(recordId: string): Promise<void> {
    const assets = this.db
      .select({ storageKey: workspaceAssets.storageKey })
      .from(workspaceAssets)
      .where(eq(workspaceAssets.recordId, recordId))
      .all()
    for (const asset of assets) {
      const filePath = this.resolveAssetPath(asset.storageKey)
      await fs.rm(dirname(filePath), { recursive: true, force: true })
    }
  }
}

function toWorkspaceRecord(row: WorkspaceRecordRow): WorkspaceRecord {
  return {
    ...row,
    data: parseJson<Record<string, unknown>>(row.dataJson) ?? {},
  }
}

function requireText(value: string | undefined, label: string): string {
  const trimmed = value?.trim()
  if (!trimmed) throw new Error(`${label} is required`)
  return trimmed
}

function optionalText(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed || null
}

function stringifyJson(value: unknown): string {
  return JSON.stringify(value)
}

function parseJson<T>(value: string | null): T | null {
  if (!value) return null
  try {
    return JSON.parse(value) as T
  } catch {
    return null
  }
}

function slugify(value: string): string {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return slug || `field-${crypto.randomUUID().slice(0, 8)}`
}

function sanitizeFilename(value: string): string {
  const candidate = basename(value.trim())
    .replace(/[^a-zA-Z0-9._-]+/g, '_')
    .replace(/^\.+$/, '')
  return candidate || 'asset'
}

function clampInteger(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min
  return Math.min(max, Math.max(min, Math.trunc(value)))
}

function lowercaseFirst(value: string): string {
  return value.length > 0 ? `${value[0]?.toLowerCase() ?? ''}${value.slice(1)}` : value
}
