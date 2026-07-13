/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Workspace MCP tools exposed to a local ACP agent such as OpenCode.
 * Every call is scoped to the conversation id supplied by the BrowserOS
 * MCP route. Agents cannot write workspace data without a matching,
 * user-created research session.
 */

import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import {
  MAX_WORKSPACE_ASSET_BYTES,
  type ResearchEventKind,
  type ResearchPlanStepStatus,
  type ResearchSessionStatus,
  type WorkspaceStore,
} from '../../../lib/workspace/workspace-store'

const MAX_TEXT_LENGTH = 20_000
const MAX_BASE64_LENGTH = Math.ceil(MAX_WORKSPACE_ASSET_BYTES * (4 / 3)) + 256

const researchSessionStatusSchema = z.enum([
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
])

const researchPlanStepStatusSchema = z.enum([
  'pending',
  'running',
  'completed',
  'blocked',
  'skipped',
  'failed',
])

const researchEventKindSchema = z.enum([
  'activity',
  'reasoning-summary',
  'decision',
  'tool-call',
  'error',
  'checkpoint',
])

interface WorkspaceMcpScope {
  store: WorkspaceStore
  conversationId: string
}

type ToolResult = {
  content: Array<{ type: 'text'; text: string }>
  isError?: boolean
}

type WorkspaceMcpRegister = (
  name: string,
  config: { description: string; inputSchema: unknown },
  handler: (args: Record<string, unknown>) => Promise<ToolResult>,
) => void

function success(value: unknown): ToolResult {
  return {
    content: [{ type: 'text', text: JSON.stringify(value) }],
  }
}

function failure(error: unknown): ToolResult {
  return {
    isError: true,
    content: [
      {
        type: 'text',
        text: error instanceof Error ? error.message : String(error),
      },
    ],
  }
}

async function requireSession(scope: WorkspaceMcpScope) {
  if (!scope.conversationId || scope.conversationId === 'ephemeral') {
    throw new Error(
      'Workspace tools require an active BrowserOS research conversation.',
    )
  }
  const session = await scope.store.getResearchSessionByConversationId(
    scope.conversationId,
  )
  if (!session) {
    throw new Error(
      'No research session is linked to this conversation. Start the task from BrowserOS first.',
    )
  }
  if (['completed', 'failed', 'cancelled'].includes(session.status)) {
    throw new Error(
      `Research session is ${session.status}; start a new session before writing workspace data.`,
    )
  }
  return session
}

async function requireDatabase(
  scope: WorkspaceMcpScope,
  session: Awaited<ReturnType<typeof requireSession>>,
  databaseId: string,
) {
  const database = await scope.store.getDatabase(databaseId)
  if (!database) throw new Error(`Workspace database not found: ${databaseId}`)
  if (
    session.collectionId &&
    database.collectionId &&
    session.collectionId !== database.collectionId
  ) {
    throw new Error('The selected database is outside this research collection.')
  }
  return database
}

async function requireSessionSource(
  scope: WorkspaceMcpScope,
  sessionId: string,
  sourceId: string,
) {
  const source = await scope.store.getSource(sourceId)
  if (!source || source.sessionId !== sessionId) {
    throw new Error('Source does not belong to the active research session.')
  }
  return source
}

async function requireSessionRecord(
  scope: WorkspaceMcpScope,
  sessionId: string,
  recordId: string,
) {
  const record = await scope.store.getRecord(recordId)
  if (!record || record.sessionId !== sessionId) {
    throw new Error('Record does not belong to the active research session.')
  }
  return record
}

function decodeBase64(value: string): Uint8Array {
  const encoded = value.includes(',') ? value.slice(value.indexOf(',') + 1) : value
  if (!encoded || encoded.length > MAX_BASE64_LENGTH || !/^[A-Za-z0-9+/=\s]+$/.test(encoded)) {
    throw new Error('Asset data must be valid base64 within the 25 MB limit.')
  }
  return Uint8Array.from(Buffer.from(encoded, 'base64'))
}

/** Registers scoped workspace/research tools on the BrowserOS MCP server. */
export function registerWorkspaceMcpTools(
  server: McpServer,
  scope: WorkspaceMcpScope,
): void {
  const register = server.registerTool.bind(server) as unknown as WorkspaceMcpRegister

  register(
    'research_get_session',
    {
      description:
        'Get the active BrowserOS research goal, plan, status, and recent activity. Call this before planning or saving results.',
      inputSchema: z.object({}),
    },
    async () => {
      try {
        const session = await requireSession(scope)
        return success({
          session: {
            id: session.id,
            goal: session.goal,
            status: session.status,
            databaseId: session.databaseId,
            collectionId: session.collectionId,
          },
          plan: session.plan,
          recentActivity: session.events.slice(-20),
        })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'workspace_list_databases',
    {
      description:
        'List the databases available to the active research collection, including field definitions. Use this before saving records.',
      inputSchema: z.object({}),
    },
    async () => {
      try {
        const session = await requireSession(scope)
        const databases = await scope.store.listDatabases(session.collectionId ?? undefined)
        const detailed = await Promise.all(
          databases.map(async (database) => {
            const withFields = await scope.store.getDatabase(database.id)
            return { ...database, fields: withFields?.fields ?? [] }
          }),
        )
        return success({
          selectedDatabaseId: session.databaseId,
          databases: detailed,
        })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'workspace_select_database',
    {
      description:
        'Select an existing workspace database as the destination for this research session.',
      inputSchema: z.object({ databaseId: z.string().min(1) }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const databaseId = (args as { databaseId: string }).databaseId
        await requireDatabase(scope, session, databaseId)
        const updated = await scope.store.updateResearchSession(session.id, {
          databaseId,
        })
        return success({ databaseId, sessionId: updated?.id })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'workspace_create_database',
    {
      description:
        'Create a workspace database for the active research session and select it as the save destination.',
      inputSchema: z.object({
        name: z.string().min(1).max(200),
        description: z.string().max(MAX_TEXT_LENGTH).optional(),
      }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const input = args as { name: string; description?: string }
        const database = await scope.store.createDatabase({
          name: input.name,
          description: input.description,
          collectionId: session.collectionId ?? undefined,
        })
        await scope.store.updateResearchSession(session.id, {
          databaseId: database.id,
        })
        return success({ database })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'workspace_create_source',
    {
      description:
        'Save a source URL and a short evidence excerpt for the active research session. Create one source before saving records from a page.',
      inputSchema: z.object({
        url: z.string().url(),
        title: z.string().max(500).optional(),
        excerpt: z.string().max(MAX_TEXT_LENGTH).optional(),
        contentHash: z.string().max(128).optional(),
      }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const input = args as {
          url: string
          title?: string
          excerpt?: string
          contentHash?: string
        }
        const source = await scope.store.createSource({
          ...input,
          sessionId: session.id,
        })
        return success({ source })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'workspace_create_record',
    {
      description:
        'Save one verified structured record in the selected workspace database. Include only fields supported by the source and include sourceId when available.',
      inputSchema: z.object({
        databaseId: z.string().optional(),
        title: z.string().max(500).optional(),
        data: z.record(z.string(), z.unknown()),
        sourceId: z.string().optional(),
      }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const input = args as {
          databaseId?: string
          title?: string
          data: Record<string, unknown>
          sourceId?: string
        }
        const databaseId = input.databaseId ?? session.databaseId
        if (!databaseId) {
          throw new Error(
            'Select or create a workspace database before saving records.',
          )
        }
        await requireDatabase(scope, session, databaseId)
        if (input.sourceId) {
          await requireSessionSource(scope, session.id, input.sourceId)
        }
        const record = await scope.store.createRecord({
          databaseId,
          title: input.title,
          data: input.data,
          sessionId: session.id,
          sourceId: input.sourceId,
        })
        return success({ record })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'workspace_save_asset',
    {
      description:
        'Save a screenshot, downloaded evidence file, or other asset into the active workspace session. Pass base64 data and optionally link it to a record or source.',
      inputSchema: z.object({
        filename: z.string().min(1).max(255),
        mimeType: z.string().min(1).max(200),
        dataBase64: z.string().min(1).max(MAX_BASE64_LENGTH),
        recordId: z.string().optional(),
        sourceId: z.string().optional(),
        width: z.number().int().nonnegative().optional(),
        height: z.number().int().nonnegative().optional(),
      }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const input = args as {
          filename: string
          mimeType: string
          dataBase64: string
          recordId?: string
          sourceId?: string
          width?: number
          height?: number
        }
        if (input.recordId) {
          await requireSessionRecord(scope, session.id, input.recordId)
        }
        if (input.sourceId) {
          await requireSessionSource(scope, session.id, input.sourceId)
        }
        const asset = await scope.store.createAsset({
          filename: input.filename,
          mimeType: input.mimeType,
          data: decodeBase64(input.dataBase64),
          sessionId: session.id,
          recordId: input.recordId,
          sourceId: input.sourceId,
          width: input.width,
          height: input.height,
        })
        return success({ asset })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'research_log_event',
    {
      description:
        'Append a concise, user-safe activity summary or decision to the active research session. Do not store private chain-of-thought.',
      inputSchema: z.object({
        kind: researchEventKindSchema.optional(),
        title: z.string().min(1).max(500),
        detail: z.string().max(MAX_TEXT_LENGTH).optional(),
        payload: z.record(z.string(), z.unknown()).optional(),
      }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const input = args as {
          kind?: ResearchEventKind
          title: string
          detail?: string
          payload?: Record<string, unknown>
        }
        const event = await scope.store.addResearchEvent({
          sessionId: session.id,
          kind: input.kind ?? 'activity',
          title: input.title,
          detail: input.detail,
          payload: input.payload,
        })
        return success({ event })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'research_update_plan_step',
    {
      description:
        'Update the status of one step in the active research plan after completing or blocking it.',
      inputSchema: z.object({
        stepId: z.string().min(1),
        status: researchPlanStepStatusSchema,
        error: z.string().max(MAX_TEXT_LENGTH).optional(),
      }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const input = args as {
          stepId: string
          status: ResearchPlanStepStatus
          error?: string
        }
        if (!session.plan.some((step) => step.id === input.stepId)) {
          throw new Error('Plan step does not belong to the active session.')
        }
        const step = await scope.store.updatePlanStep(input.stepId, {
          status: input.status,
          error: input.error,
        })
        return success({ step })
      } catch (error) {
        return failure(error)
      }
    },
  )

  register(
    'research_update_status',
    {
      description:
        'Update the active research session status. Mark completed only after verifying coverage, source links, and saved records.',
      inputSchema: z.object({ status: researchSessionStatusSchema }),
    },
    async (args) => {
      try {
        const session = await requireSession(scope)
        const status = (args as { status: ResearchSessionStatus }).status
        const updated = await scope.store.updateResearchSession(session.id, {
          status,
        })
        const recap =
          status === 'completed' || status === 'paused'
            ? await scope.store.buildSessionRecap(session.id)
            : null
        return success({ session: updated, recap })
      } catch (error) {
        return failure(error)
      }
    },
  )
}
