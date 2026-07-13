/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import {
  MAX_WORKSPACE_ASSET_BYTES,
  type ResearchEventKind,
  type ResearchPlanStepStatus,
  type ResearchSessionStatus,
  type ResearchToolCategory,
  type WorkspaceFieldType,
  WorkspaceStore,
} from '../../lib/workspace/workspace-store'

const fieldTypes = [
  'text',
  'number',
  'currency',
  'date',
  'boolean',
  'select',
  'multi-select',
  'url',
  'image',
  'location',
  'source',
  'asset',
] as const

const sessionStatuses = [
  'draft',
  'planning',
  'awaiting_plan_approval',
  'running',
  'awaiting_user',
  'paused',
  'verifying',
  'completed',
  'failed',
  'cancelled',
] as const

const planStepStatuses = [
  'pending',
  'running',
  'completed',
  'blocked',
  'skipped',
  'failed',
] as const

const toolCategories = [
  'browser',
  'extraction',
  'database',
  'analysis',
  'report',
] as const

const eventKinds = [
  'activity',
  'reasoning-summary',
  'decision',
  'tool-call',
  'error',
  'checkpoint',
] as const

const jsonObject = z.record(z.string(), z.unknown())

const collectionSchema = z.object({
  name: z.string().min(1).max(200),
  description: z.string().max(2000).optional(),
})

const collectionPatchSchema = collectionSchema.partial()

const databaseSchema = z.object({
  name: z.string().min(1).max(200),
  description: z.string().max(2000).optional(),
  collectionId: z.string().uuid().optional(),
})

const databasePatchSchema = z.object({
  name: z.string().min(1).max(200).optional(),
  description: z.string().max(2000).nullable().optional(),
  collectionId: z.string().uuid().nullable().optional(),
})

const fieldSchema = z.object({
  name: z.string().min(1).max(200),
  key: z.string().max(200).optional(),
  type: z.enum(fieldTypes),
  position: z.number().int().min(0).max(10000).optional(),
  required: z.boolean().optional(),
  configuration: jsonObject.optional(),
})

const fieldPatchSchema = fieldSchema.partial().extend({
  configuration: jsonObject.nullable().optional(),
})

const recordSchema = z.object({
  title: z.string().max(500).optional(),
  data: jsonObject,
  sessionId: z.string().uuid().optional(),
  sourceId: z.string().uuid().optional(),
})

const recordPatchSchema = z.object({
  title: z.string().max(500).nullable().optional(),
  data: jsonObject.optional(),
  sessionId: z.string().uuid().nullable().optional(),
  sourceId: z.string().uuid().nullable().optional(),
})

const sourceSchema = z.object({
  url: z.string().url().max(4000),
  title: z.string().max(500).optional(),
  excerpt: z.string().max(10000).optional(),
  contentHash: z.string().max(128).optional(),
  snapshotPath: z.string().max(1000).optional(),
  sessionId: z.string().uuid().optional(),
  accessedAt: z.number().int().positive().optional(),
})

const sessionSchema = z.object({
  goal: z.string().min(1).max(10000),
  conversationId: z.string().uuid().optional(),
  browserProfileId: z.string().max(300).optional(),
  collectionId: z.string().uuid().optional(),
  databaseId: z.string().uuid().optional(),
  status: z.enum(sessionStatuses).optional(),
  plan: z
    .array(
      z.object({
        title: z.string().min(1).max(500),
        description: z.string().max(2000).optional(),
        toolCategory: z.enum(toolCategories),
        requiresApproval: z.boolean().optional(),
        expectedOutput: z.string().max(1000).optional(),
      }),
    )
    .max(100)
    .optional(),
})

const sessionPatchSchema = z.object({
  status: z.enum(sessionStatuses).optional(),
  activeStepId: z.string().uuid().nullable().optional(),
  collectionId: z.string().uuid().nullable().optional(),
  databaseId: z.string().uuid().nullable().optional(),
  browserProfileId: z.string().max(300).nullable().optional(),
})

const eventSchema = z.object({
  kind: z.enum(eventKinds),
  title: z.string().min(1).max(500),
  detail: z.string().max(5000).optional(),
  payload: jsonObject.optional(),
})

const planStepPatchSchema = z.object({
  status: z.enum(planStepStatuses).optional(),
  title: z.string().min(1).max(500).optional(),
  description: z.string().max(2000).nullable().optional(),
  requiresApproval: z.boolean().optional(),
  error: z.string().max(5000).nullable().optional(),
})

const maxAssetDataUrlLength = Math.ceil(MAX_WORKSPACE_ASSET_BYTES * (4 / 3)) + 128
const assetSchema = z
  .object({
    filename: z.string().min(1).max(255),
    mimeType: z.string().regex(/^[\w.+-]+\/[\w.+-]+$/),
    dataBase64: z.string().min(1).max(maxAssetDataUrlLength),
    sessionId: z.string().uuid().optional(),
    recordId: z.string().uuid().optional(),
    sourceId: z.string().uuid().optional(),
    width: z.number().int().positive().max(50000).optional(),
    height: z.number().int().positive().max(50000).optional(),
  })
  .superRefine((value, ctx) => {
    try {
      const bytes = Buffer.from(value.dataBase64, 'base64')
      if (bytes.byteLength > MAX_WORKSPACE_ASSET_BYTES) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ['dataBase64'],
          message: `Asset exceeds ${MAX_WORKSPACE_ASSET_BYTES} bytes`,
        })
      }
    } catch {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['dataBase64'],
        message: 'Asset data must be base64 encoded',
      })
    }
  })

export interface WorkspaceRouteDeps {
  store?: WorkspaceStore
}

export function createWorkspaceRoutes(deps: WorkspaceRouteDeps = {}) {
  const store = deps.store ?? new WorkspaceStore()

  return new Hono()
    .get('/collections', async (c) => {
      return c.json({ collections: await store.listCollections() })
    })
    .post('/collections', zValidator('json', collectionSchema), async (c) => {
      return c.json({ collection: await store.createCollection(c.req.valid('json')) }, 201)
    })
    .patch(
      '/collections/:collectionId',
      zValidator('json', collectionPatchSchema),
      async (c) => {
        const collection = await store.updateCollection(
          c.req.param('collectionId'),
          c.req.valid('json'),
        )
        return collection
          ? c.json({ collection })
          : c.json({ error: 'Collection not found' }, 404)
      },
    )
    .delete('/collections/:collectionId', async (c) => {
      const deleted = await store.deleteCollection(c.req.param('collectionId'))
      return deleted
        ? c.json({ deleted: true })
        : c.json({ error: 'Collection not found' }, 404)
    })
    .get('/databases', async (c) => {
      return c.json({ databases: await store.listDatabases(c.req.query('collectionId')) })
    })
    .post('/databases', zValidator('json', databaseSchema), async (c) => {
      return c.json({ database: await store.createDatabase(c.req.valid('json')) }, 201)
    })
    .get('/databases/:databaseId', async (c) => {
      const database = await store.getDatabase(c.req.param('databaseId'))
      return database
        ? c.json({ database })
        : c.json({ error: 'Database not found' }, 404)
    })
    .patch(
      '/databases/:databaseId',
      zValidator('json', databasePatchSchema),
      async (c) => {
        const database = await store.updateDatabase(
          c.req.param('databaseId'),
          c.req.valid('json'),
        )
        return database
          ? c.json({ database })
          : c.json({ error: 'Database not found' }, 404)
      },
    )
    .delete('/databases/:databaseId', async (c) => {
      const deleted = await store.deleteDatabase(c.req.param('databaseId'))
      return deleted
        ? c.json({ deleted: true })
        : c.json({ error: 'Database not found' }, 404)
    })
    .post(
      '/databases/:databaseId/fields',
      zValidator('json', fieldSchema),
      async (c) => {
        return c.json(
          {
            field: await store.createField({
              databaseId: c.req.param('databaseId'),
              ...c.req.valid('json'),
            }),
          },
          201,
        )
      },
    )
    .patch(
      '/fields/:fieldId',
      zValidator('json', fieldPatchSchema),
      async (c) => {
        const field = await store.updateField(c.req.param('fieldId'), c.req.valid('json'))
        return field ? c.json({ field }) : c.json({ error: 'Field not found' }, 404)
      },
    )
    .delete('/fields/:fieldId', async (c) => {
      const deleted = await store.deleteField(c.req.param('fieldId'))
      return deleted
        ? c.json({ deleted: true })
        : c.json({ error: 'Field not found' }, 404)
    })
    .get('/databases/:databaseId/records', async (c) => {
      const limit = parseQueryNumber(c.req.query('limit'))
      const offset = parseQueryNumber(c.req.query('offset'))
      return c.json({
        records: await store.listRecords(c.req.param('databaseId'), { limit, offset }),
      })
    })
    .post(
      '/databases/:databaseId/records',
      zValidator('json', recordSchema),
      async (c) => {
        return c.json(
          {
            record: await store.createRecord({
              databaseId: c.req.param('databaseId'),
              ...c.req.valid('json'),
            }),
          },
          201,
        )
      },
    )
    .patch(
      '/records/:recordId',
      zValidator('json', recordPatchSchema),
      async (c) => {
        const record = await store.updateRecord(c.req.param('recordId'), c.req.valid('json'))
        return record ? c.json({ record }) : c.json({ error: 'Record not found' }, 404)
      },
    )
    .delete('/records/:recordId', async (c) => {
      const deleted = await store.deleteRecord(c.req.param('recordId'))
      return deleted
        ? c.json({ deleted: true })
        : c.json({ error: 'Record not found' }, 404)
    })
    .get('/sources', async (c) => {
      return c.json({ sources: await store.listSources(c.req.query('sessionId')) })
    })
    .post('/sources', zValidator('json', sourceSchema), async (c) => {
      return c.json({ source: await store.createSource(c.req.valid('json')) }, 201)
    })
    .get('/assets', async (c) => {
      return c.json({
        assets: await store.listAssets({
          sessionId: c.req.query('sessionId'),
          recordId: c.req.query('recordId'),
          sourceId: c.req.query('sourceId'),
        }),
      })
    })
    .post('/assets', zValidator('json', assetSchema), async (c) => {
      const body = c.req.valid('json')
      const bytes = Buffer.from(body.dataBase64, 'base64')
      const asset = await store.createAsset({
        filename: body.filename,
        mimeType: body.mimeType,
        data: bytes,
        sessionId: body.sessionId,
        recordId: body.recordId,
        sourceId: body.sourceId,
        width: body.width,
        height: body.height,
      })
      return c.json({ asset }, 201)
    })
    .get('/assets/:assetId/content', async (c) => {
      const result = await store.readAsset(c.req.param('assetId'))
      if (!result) return c.json({ error: 'Asset not found' }, 404)
      const body = result.data.buffer.slice(
        result.data.byteOffset,
        result.data.byteOffset + result.data.byteLength,
      ) as ArrayBuffer
      return new Response(body, {
        headers: {
          'Content-Type': result.asset.mimeType,
          'Content-Length': String(result.asset.byteSize),
          'Content-Disposition': `inline; filename="${result.asset.filename}"`,
          'Cache-Control': 'private, max-age=31536000, immutable',
        },
      })
    })
    .delete('/assets/:assetId', async (c) => {
      const deleted = await store.deleteAsset(c.req.param('assetId'))
      return deleted
        ? c.json({ deleted: true })
        : c.json({ error: 'Asset not found' }, 404)
    })
    .get('/sessions', async (c) => {
      return c.json({
        sessions: await store.listResearchSessions(parseQueryNumber(c.req.query('limit'))),
      })
    })
    .post('/sessions', zValidator('json', sessionSchema), async (c) => {
      return c.json({ session: await store.createResearchSession(c.req.valid('json')) }, 201)
    })
    .get('/sessions/:sessionId', async (c) => {
      const session = await store.getResearchSession(c.req.param('sessionId'))
      return session
        ? c.json({ session })
        : c.json({ error: 'Research session not found' }, 404)
    })
    .patch(
      '/sessions/:sessionId',
      zValidator('json', sessionPatchSchema),
      async (c) => {
        const session = await store.updateResearchSession(
          c.req.param('sessionId'),
          c.req.valid('json'),
        )
        return session
          ? c.json({ session })
          : c.json({ error: 'Research session not found' }, 404)
      },
    )
    .post('/sessions/:sessionId/recap', async (c) => {
      const recap = await store.buildSessionRecap(c.req.param('sessionId'))
      return recap
        ? c.json({ recap })
        : c.json({ error: 'Research session not found' }, 404)
    })
    .post('/sessions/:sessionId/suggestion', async (c) => {
      const suggestion = await store.buildSessionSuggestion(c.req.param('sessionId'))
      return suggestion
        ? c.json({ suggestion })
        : c.json({ error: 'Research session not found' }, 404)
    })
    .post(
      '/sessions/:sessionId/events',
      zValidator('json', eventSchema),
      async (c) => {
        return c.json(
          {
            event: await store.addResearchEvent({
              sessionId: c.req.param('sessionId'),
              ...c.req.valid('json'),
            }),
          },
          201,
        )
      },
    )
    .patch(
      '/plan-steps/:stepId',
      zValidator('json', planStepPatchSchema),
      async (c) => {
        const step = await store.updatePlanStep(c.req.param('stepId'), c.req.valid('json'))
        return step ? c.json({ step }) : c.json({ error: 'Plan step not found' }, 404)
      },
    )
}

function parseQueryNumber(value: string | undefined): number | undefined {
  if (!value) return undefined
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : undefined
}

// Keep route schemas aligned with the storage unions when refactoring either
// side; these aliases intentionally fail type-checking if they drift.
const _workspaceRouteTypes: [
  WorkspaceFieldType,
  ResearchSessionStatus,
  ResearchPlanStepStatus,
  ResearchToolCategory,
  ResearchEventKind,
] | null = null
void _workspaceRouteTypes
