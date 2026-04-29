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
import {
  AcpxRuntime,
  type OpenclawGatewayAccessor,
} from '../../lib/agents/acpx-runtime'
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
  type OpenClawProvisioner,
  OpenClawProvisionerUnavailableError,
  UnknownAgentError,
} from '../services/agents/agent-harness-service'
import type { OpenClawGatewayChatClient } from '../services/openclaw/openclaw-gateway-chat-client'
import type { Env } from '../types'
import { resolveBrowserContextPageIds } from '../utils/resolve-browser-context-page-ids'

type AgentRouteService = {
  listAgents(): Promise<AgentDefinition[]>
  createAgent(input: {
    name: string
    adapter: AgentAdapter
    modelId?: string
    reasoningEffort?: string
    providerType?: string
    providerName?: string
    baseUrl?: string
    apiKey?: string
    supportsImages?: boolean
  }): Promise<AgentDefinition>
  getAgent(agentId: string): Promise<AgentDefinition | null>
  deleteAgent(agentId: string): Promise<boolean>
  getHistory(agentId: string): Promise<AgentHistoryPage>
  send(input: {
    agentId: string
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    signal?: AbortSignal
  }): Promise<ReadableStream<AgentStreamEvent>>
}

type AgentRouteDeps = {
  service?: AgentRouteService
  runtime?: AgentRuntime
  browser?: Pick<Browser, 'resolveTabIds'>
  browserosServerPort?: number
  /**
   * Required when an `openclaw` adapter agent is in use; harmless when
   * absent. Forwarded to the AcpxRuntime so it can spawn `openclaw acp`
   * inside the gateway container.
   */
  openclawGateway?: OpenclawGatewayAccessor
  /**
   * Optional. Enables the image-attachment carve-out for OpenClaw
   * agents — image-bearing turns route through the gateway HTTP
   * `/v1/chat/completions` instead of the ACP bridge (which drops
   * image content blocks).
   */
  openclawGatewayChat?: OpenClawGatewayChatClient
  /**
   * Required to dual-create/delete `openclaw` adapter agents on the
   * gateway side. Without this, openclaw create requests fail with 503.
   */
  openclawProvisioner?: OpenClawProvisioner
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
    new AgentHarnessService({
      browserosServerPort: deps.browserosServerPort,
      openclawGateway: deps.openclawGateway,
      openclawGatewayChat: deps.openclawGatewayChat,
      openclawProvisioner: deps.openclawProvisioner,
    })
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
          openclawGateway: deps.openclawGateway,
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
          attachments: parsed.attachments,
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
      providerType?: string
      providerName?: string
      baseUrl?: string
      apiKey?: string
      supportsImages?: boolean
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

  // OpenClaw agents resolve their model from the gateway-side provider
  // config rather than from the harness catalog. Skip catalog model
  // validation for that adapter; everything else still uses the catalog.
  if (
    record.adapter !== 'openclaw' &&
    !isSupportedAgentModel(record.adapter, modelId)
  ) {
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
    providerType: readOptionalTrimmedString(record, 'providerType'),
    providerName: readOptionalTrimmedString(record, 'providerName'),
    baseUrl: readOptionalTrimmedString(record, 'baseUrl'),
    apiKey: readOptionalTrimmedString(record, 'apiKey'),
    supportsImages:
      typeof record.supportsImages === 'boolean'
        ? record.supportsImages
        : undefined,
  }
}

/**
 * Image attachment forwarded from the chat composer. The dataUrl is a
 * `data:<mime>;base64,<payload>` string the composer pre-encoded; the
 * harness strips the prefix and hands raw base64 to acpx, which builds
 * the ACP `image` content block.
 */
export interface InboundImageAttachment {
  mediaType: string
  data: string
}

// Defense-in-depth caps on chat-body image attachments. The composer
// already enforces these client-side (see `lib/attachments.ts`) but
// `/agents/:id/chat` accepts direct curl/script callers too, so the
// server has to validate independently.
const MAX_CHAT_ATTACHMENTS = 10
const MAX_IMAGE_BYTES = 5 * 1024 * 1024 // 5 MB raw, post-decode
// data: URLs encode bytes as base64 (~4/3 inflation) plus the
// `data:<mime>;base64,` prefix; cap the encoded string against that
// rather than 2× the raw budget.
const MAX_IMAGE_DATA_URL_LENGTH = Math.ceil(MAX_IMAGE_BYTES * (4 / 3)) + 100
const ALLOWED_IMAGE_MEDIA_TYPES = new Set([
  'image/png',
  'image/jpeg',
  'image/jpg',
  'image/webp',
  'image/gif',
])

async function parseChatBody(
  c: Context<Env>,
): Promise<
  { message: string; attachments: InboundImageAttachment[] } | { error: string }
> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  const message =
    typeof body.value.message === 'string' ? body.value.message.trim() : ''
  const attachmentsRaw = Array.isArray(body.value.attachments)
    ? body.value.attachments
    : []
  if (attachmentsRaw.length > MAX_CHAT_ATTACHMENTS) {
    return {
      error: `at most ${MAX_CHAT_ATTACHMENTS} attachments are allowed per message`,
    }
  }
  const attachments: InboundImageAttachment[] = []
  for (const entry of attachmentsRaw) {
    if (!entry || typeof entry !== 'object') {
      return { error: 'invalid attachment entry' }
    }
    const record = entry as Record<string, unknown>
    if (record.kind !== 'image') {
      return { error: 'attachment kind must be "image"' }
    }
    const mediaType =
      typeof record.mediaType === 'string' ? record.mediaType : ''
    const dataUrl = typeof record.dataUrl === 'string' ? record.dataUrl : ''
    if (!ALLOWED_IMAGE_MEDIA_TYPES.has(mediaType)) {
      return {
        error: `unsupported image type: ${mediaType || 'unknown'}`,
      }
    }
    if (!dataUrl.startsWith('data:')) {
      return { error: 'image attachment must include a data: URL' }
    }
    if (dataUrl.length > MAX_IMAGE_DATA_URL_LENGTH) {
      return { error: `image exceeds ${MAX_IMAGE_BYTES} bytes` }
    }
    // Strip the `data:<mime>;base64,` prefix — ACP image blocks carry
    // raw base64 plus the mime type as separate fields.
    const commaIdx = dataUrl.indexOf(',')
    const data = commaIdx >= 0 ? dataUrl.slice(commaIdx + 1) : dataUrl
    if (!data) {
      return { error: 'image attachment payload is empty' }
    }
    attachments.push({ mediaType, data })
  }
  if (!message && attachments.length === 0) {
    return { error: 'Message is required' }
  }
  return { message, attachments }
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
  if (err instanceof OpenClawProvisionerUnavailableError) {
    return c.json({ error: err.message }, 503)
  }
  const message = err instanceof Error ? err.message : String(err)
  return c.json({ error: message }, 500)
}
