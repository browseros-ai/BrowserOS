/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { AGENT_HARNESS_LIMITS } from '@browseros/shared/constants/limits'
import {
  type BrowserContext,
  BrowserContextSchema,
} from '@browseros/shared/schemas/browser-context'
import { type Context, Hono } from 'hono'
import { stream } from 'hono/streaming'
import { formatUserMessage } from '../../agent/format-message'
import type { Browser } from '../../browser/browser'
import { createAcpUIMessageStreamResponse } from '../../lib/agents/acp-ui-message-stream'
import { AcpxRuntime } from '../../lib/agents/acpx-runtime'
import {
  AGENT_ADAPTER_CATALOG,
  getAgentAdapterDescriptor,
  isAgentAdapter,
  isSupportedAgentModel,
  isSupportedReasoningEffort,
  resolveDefaultModelId,
  resolveDefaultReasoningEffort,
} from '../../lib/agents/agent-catalog'
import type {
  AgentAdapter,
  AgentDefinition,
} from '../../lib/agents/agent-types'
import type {
  AgentHistoryPage,
  AgentRuntime,
  AgentStreamEvent,
} from '../../lib/agents/types'
import {
  AgentHarnessService,
  UnknownAgentError,
} from '../services/agents/agent-harness-service'
import type { Env } from '../types'
import { resolveBrowserContextPageIds } from '../utils/resolve-browser-context-page-ids'

type AgentRouteService = {
  listAgents(): Promise<AgentDefinition[]>
  createAgent(input: {
    name: string
    adapter: AgentAdapter
    modelId?: string
    reasoningEffort?: string
  }): Promise<AgentDefinition>
  getAgent(agentId: string): Promise<AgentDefinition | null>
  deleteAgent(agentId: string): Promise<boolean>
  getHistory(agentId: string): Promise<AgentHistoryPage>
  send(input: {
    agentId: string
    message: string
    signal?: AbortSignal
  }): Promise<ReadableStream<AgentStreamEvent>>
}

type AgentRouteDeps = {
  service?: AgentRouteService
  runtime?: AgentRuntime
  browser?: Pick<Browser, 'resolveTabIds'>
  browserosServerPort?: number
}

type SidepanelAcpChatRequest = {
  conversationId: string
  adapter: AgentAdapter
  modelId: string
  reasoningEffort: string
  message: string
  browserContext?: BrowserContext
  selectedText?: string
  selectedTextSource?: { url: string; title: string }
  userSystemPrompt?: string
  userWorkingDir?: string
}

export function createAgentRoutes(deps: AgentRouteDeps = {}) {
  const service =
    deps.service ??
    new AgentHarnessService({ browserosServerPort: deps.browserosServerPort })
  let sidepanelRuntime = deps.runtime

  return new Hono<Env>()
    .get('/adapters', (c) => c.json({ adapters: AGENT_ADAPTER_CATALOG }))
    .get('/', async (c) => c.json({ agents: await service.listAgents() }))
    .post('/', async (c) => {
      const parsed = await parseCreateAgentBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)
      try {
        return c.json({ agent: await service.createAgent(parsed) })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .post('/sidepanel/chat', async (c) => {
      const parsed = await parseSidepanelAcpChatBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)

      let browserContext = parsed.browserContext
      if (deps.browser) {
        browserContext = await resolveBrowserContextPageIds(
          deps.browser,
          browserContext,
        )
      }

      const userContent = formatUserMessage(
        parsed.message,
        browserContext,
        parsed.selectedText,
        parsed.selectedTextSource,
      )
      const message = parsed.userSystemPrompt?.trim()
        ? `${parsed.userSystemPrompt.trim()}\n\n${userContent}`
        : userContent
      const agent = buildSidepanelAcpAgent(parsed)

      try {
        sidepanelRuntime ??= new AcpxRuntime({
          browserosServerPort: deps.browserosServerPort,
        })
        const eventStream = await sidepanelRuntime.send({
          agent,
          sessionId: 'main',
          sessionKey: agent.sessionKey,
          message,
          permissionMode: agent.permissionMode,
          cwd: parsed.userWorkingDir,
          signal: c.req.raw.signal,
        })
        return createAcpUIMessageStreamResponse(eventStream)
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .get('/:agentId', async (c) => {
      try {
        const agent = await service.getAgent(c.req.param('agentId'))
        if (!agent) return c.json({ error: 'Unknown agent' }, 404)
        return c.json({ agent })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .delete('/:agentId', async (c) => {
      try {
        return c.json({
          success: await service.deleteAgent(c.req.param('agentId')),
        })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .get('/:agentId/sessions/main/history', async (c) => {
      try {
        return c.json(await service.getHistory(c.req.param('agentId')))
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .post('/:agentId/chat', async (c) => {
      const agentId = c.req.param('agentId')
      const parsed = await parseChatBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)

      let eventStream: ReadableStream<AgentStreamEvent>
      try {
        eventStream = await service.send({
          agentId,
          message: parsed.message,
          signal: c.req.raw.signal,
        })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }

      c.header('Content-Type', 'text/event-stream')
      c.header('Cache-Control', 'no-cache')
      c.header('X-Session-Id', 'main')

      return stream(c, async (s) => {
        const reader = eventStream.getReader()
        const encoder = new TextEncoder()
        let completed = false
        try {
          while (true) {
            const { done, value } = await reader.read()
            if (done) break
            await s.write(encoder.encode(`data: ${JSON.stringify(value)}\n\n`))
          }
          await s.write(encoder.encode('data: [DONE]\n\n'))
          completed = true
        } finally {
          if (completed) {
            reader.releaseLock()
          } else {
            await reader.cancel('BrowserOS HTTP stream ended').catch(() => {})
          }
        }
      })
    })
}

async function parseCreateAgentBody(c: Context<Env>): Promise<
  | {
      name: string
      adapter: AgentAdapter
      modelId?: string
      reasoningEffort?: string
    }
  | { error: string }
> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  const record = body.value
  const name = typeof record.name === 'string' ? record.name.trim() : ''
  if (!name) return { error: 'Name is required' }
  if (name.length > AGENT_HARNESS_LIMITS.AGENT_NAME_MAX_CHARS) {
    return {
      error: `Name must be ${AGENT_HARNESS_LIMITS.AGENT_NAME_MAX_CHARS} characters or fewer`,
    }
  }
  if (!isAgentAdapter(record.adapter)) {
    return { error: 'Invalid adapter' }
  }

  const modelId =
    typeof record.modelId === 'string' && record.modelId.trim()
      ? record.modelId.trim()
      : undefined
  const reasoningEffort =
    typeof record.reasoningEffort === 'string' && record.reasoningEffort.trim()
      ? record.reasoningEffort.trim()
      : undefined

  if (!isSupportedAgentModel(record.adapter, modelId)) {
    return { error: 'Invalid modelId' }
  }
  if (!isSupportedReasoningEffort(record.adapter, reasoningEffort)) {
    return { error: 'Invalid reasoningEffort' }
  }

  return {
    name,
    adapter: record.adapter,
    modelId,
    reasoningEffort,
  }
}

async function parseChatBody(
  c: Context<Env>,
): Promise<{ message: string } | { error: string }> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  const message =
    typeof body.value.message === 'string' ? body.value.message.trim() : ''
  return message ? { message } : { error: 'Message is required' }
}

async function parseSidepanelAcpChatBody(
  c: Context<Env>,
): Promise<SidepanelAcpChatRequest | { error: string }> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  const record = body.value

  const conversationId = readOptionalTrimmedString(record, 'conversationId')
  if (!conversationId || !isUuid(conversationId)) {
    return { error: 'conversationId must be a UUID' }
  }
  if (!isAgentAdapter(record.adapter)) {
    return { error: 'Invalid adapter' }
  }

  const modelId =
    readOptionalTrimmedString(record, 'modelId') ??
    resolveDefaultModelId(record.adapter)
  const reasoningEffort =
    readOptionalTrimmedString(record, 'reasoningEffort') ??
    resolveDefaultReasoningEffort(record.adapter)

  if (!isSupportedAgentModel(record.adapter, modelId)) {
    return { error: 'Invalid modelId' }
  }
  if (!isSupportedReasoningEffort(record.adapter, reasoningEffort)) {
    return { error: 'Invalid reasoningEffort' }
  }

  const message = readOptionalTrimmedString(record, 'message')
  if (!message) return { error: 'Message is required' }

  const browserContext = parseBrowserContext(record.browserContext)
  if ('error' in browserContext) return browserContext

  const selectedText = readOptionalString(record, 'selectedText')
  const selectedTextSource = parseSelectedTextSource(record.selectedTextSource)
  if ('error' in selectedTextSource) return selectedTextSource

  return {
    conversationId,
    adapter: record.adapter,
    modelId,
    reasoningEffort,
    message,
    browserContext: browserContext.value,
    selectedText,
    selectedTextSource: selectedTextSource.value,
    userSystemPrompt: readOptionalString(record, 'userSystemPrompt'),
    userWorkingDir: readOptionalTrimmedString(record, 'userWorkingDir'),
  }
}

function buildSidepanelAcpAgent(
  request: SidepanelAcpChatRequest,
): AgentDefinition {
  const now = Date.now()
  const descriptor = getAgentAdapterDescriptor(request.adapter)
  const sessionKey = [
    'sidepanel',
    request.conversationId,
    request.adapter,
    request.modelId,
    request.reasoningEffort,
  ].join(':')

  return {
    id: `sidepanel:${request.conversationId}`,
    name: descriptor?.name ?? request.adapter,
    adapter: request.adapter,
    modelId: request.modelId,
    reasoningEffort: request.reasoningEffort,
    permissionMode: 'approve-all',
    sessionKey,
    createdAt: now,
    updatedAt: now,
  }
}

function parseBrowserContext(
  value: unknown,
): { value?: BrowserContext } | { error: string } {
  if (value === undefined) return { value: undefined }
  const parsed = BrowserContextSchema.safeParse(value)
  return parsed.success
    ? { value: parsed.data }
    : { error: 'Invalid browserContext' }
}

function parseSelectedTextSource(
  value: unknown,
): { value?: { url: string; title: string } } | { error: string } {
  if (value === undefined) return { value: undefined }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return { error: 'Invalid selectedTextSource' }
  }
  const record = value as Record<string, unknown>
  return typeof record.url === 'string' && typeof record.title === 'string'
    ? { value: { url: record.url, title: record.title } }
    : { error: 'Invalid selectedTextSource' }
}

function readOptionalString(
  record: Record<string, unknown>,
  key: string,
): string | undefined {
  return typeof record[key] === 'string' ? record[key] : undefined
}

function readOptionalTrimmedString(
  record: Record<string, unknown>,
  key: string,
): string | undefined {
  const value = readOptionalString(record, key)?.trim()
  return value || undefined
}

function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
    value,
  )
}

async function readJsonBody(
  c: Context<Env>,
): Promise<{ value: Record<string, unknown> } | { error: string }> {
  let body: unknown
  try {
    body = await c.req.json()
  } catch {
    return { error: 'Invalid JSON body' }
  }

  if (!body || typeof body !== 'object' || Array.isArray(body)) {
    return { error: 'JSON object body is required' }
  }
  return { value: body as Record<string, unknown> }
}

function handleAgentRouteError(c: Context<Env>, err: unknown) {
  if (err instanceof UnknownAgentError) {
    return c.json({ error: err.message }, 404)
  }
  const message = err instanceof Error ? err.message : String(err)
  return c.json({ error: message }, 500)
}
