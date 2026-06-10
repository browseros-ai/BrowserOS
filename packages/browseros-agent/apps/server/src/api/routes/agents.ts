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
import { createAcpUIMessageStreamResponse } from '../../lib/agents/acp/ui-message-stream'
import {
  AGENT_ADAPTER_CATALOG,
  isAgentAdapter,
  isSupportedAgentModel,
  isSupportedReasoningEffort,
} from '../../lib/agents/adapters/catalog'
import { AdapterHealthChecker } from '../../lib/agents/adapters/health'
import {
  type AgentAdapter,
  type AgentDefinition,
  type AgentSessionId,
  MAIN_AGENT_SESSION_ID,
} from '../../lib/agents/agent-types'
import type {
  ActiveTurnInfo,
  TurnFrame,
} from '../../lib/agents/turns/active-turn-registry'
import type { AgentHistoryPage, AgentStreamEvent } from '../../lib/agents/types'
import {
  type AgentDefinitionWithActivity,
  AgentHarnessService,
  HermesProviderConfigInvalidError,
  InvalidAgentUpdateError,
  MessageQueueFullError,
  type QueuedMessage,
  TurnAlreadyActiveError,
  UnknownAgentError,
} from '../services/agents/agent-harness-service'
import type { Env } from '../types'
import { resolveBrowserContextPageIds } from '../utils/resolve-browser-context-page-ids'

type AgentRouteService = {
  listAgents(): Promise<AgentDefinition[]>
  listAgentsWithActivity(): Promise<AgentDefinitionWithActivity[]>
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
  updateAgent(
    agentId: string,
    patch: { name?: string; pinned?: boolean },
  ): Promise<AgentDefinition | null>
  getHistory(
    agentId: string,
    sessionId?: AgentSessionId,
  ): Promise<AgentHistoryPage>
  startTurn(input: {
    agentId: string
    sessionId?: AgentSessionId
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    cwd?: string
  }): Promise<{ turnId: string; frames: ReadableStream<TurnFrame> }>
  attachTurn(input: {
    turnId: string
    lastSeq?: number
  }): ReadableStream<TurnFrame> | null
  getActiveTurn(
    agentId: string,
    sessionId?: AgentSessionId,
  ): ActiveTurnInfo | null
  cancelTurn(input: {
    agentId: string
    sessionId?: AgentSessionId
    turnId?: string
    reason?: string
  }): boolean
  enqueueMessage(input: {
    agentId: string
    sessionId?: AgentSessionId
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
  }): Promise<QueuedMessage>
  removeQueuedMessage(input: {
    agentId: string
    messageId: string
  }): Promise<boolean>
  listQueuedMessages(agentId: string): Promise<QueuedMessage[]>
}

type AgentRouteDeps = {
  service?: AgentRouteService
  browser?: Pick<Browser, 'resolveTabIds'>
  browserosServerPort?: number
  resourcesDir?: string
  /** Optional override; defaults to a fresh in-memory checker. */
  adapterHealth?: Pick<AdapterHealthChecker, 'getHealth'>
  onTurnLifecycle?: import('../services/agents/agent-harness-service').TurnLifecycleListener
}

type SidepanelAgentChatRequest = {
  conversationId: string
  agentSessionId: AgentSessionId
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
      resourcesDir: deps.resourcesDir,
    })
  if (deps.onTurnLifecycle && service instanceof AgentHarnessService) {
    service.onTurnLifecycle(deps.onTurnLifecycle)
  }
  // One checker per route mount. Cached probes refresh every 5min;
  // tests can swap in an alternate via deps if needed.
  const adapterHealth =
    deps.adapterHealth ??
    new AdapterHealthChecker({
      hostDetectionOptions: { resourcesDir: deps.resourcesDir },
    })

  return new Hono<Env>()
    .get('/adapters', async (c) => {
      const adapters = await Promise.all(
        AGENT_ADAPTER_CATALOG.map(async (descriptor) => ({
          ...descriptor,
          health: await adapterHealth.getHealth(descriptor.id),
        })),
      )
      return c.json({ adapters })
    })
    .get('/', async (c) => {
      const agents = await service.listAgentsWithActivity()
      return c.json({ agents })
    })
    .post('/', async (c) => {
      const parsed = await parseCreateAgentBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)
      try {
        return c.json({ agent: await service.createAgent(parsed) })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .post('/:agentId/sidepanel/chat', async (c) => {
      const agentId = c.req.param('agentId')
      const parsed = await parseSidepanelAgentChatBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)

      try {
        const agent = await service.getAgent(agentId)
        if (!agent) return c.json({ error: 'Unknown agent' }, 404)

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

        let started: { turnId: string; frames: ReadableStream<TurnFrame> }
        try {
          started = await service.startTurn({
            agentId: agent.id,
            sessionId: parsed.agentSessionId,
            message,
            cwd: parsed.userWorkingDir,
          })
        } catch (err) {
          if (err instanceof TurnAlreadyActiveError) {
            return c.json(
              {
                error: 'Turn already active',
                turnId: err.turnId,
                attachUrl: buildChatStreamAttachUrl({
                  agentId: agent.id,
                  sessionId: parsed.agentSessionId,
                  turnId: err.turnId,
                }),
              },
              409,
            )
          }
          throw err
        }

        let didRequestCancel = false
        const cancelStartedTurn = () => {
          if (didRequestCancel) return
          didRequestCancel = true
          service.cancelTurn({
            agentId: agent.id,
            sessionId: parsed.agentSessionId,
            turnId: started.turnId,
            reason: 'sidepanel stream cancelled',
          })
        }
        if (c.req.raw.signal.aborted) {
          cancelStartedTurn()
        } else {
          c.req.raw.signal.addEventListener('abort', cancelStartedTurn, {
            once: true,
          })
        }

        const events = turnFramesToAgentEvents(started.frames, {
          onCancel: cancelStartedTurn,
        })

        return createAcpUIMessageStreamResponse(events, {
          headers: {
            'X-Session-Id': parsed.agentSessionId,
            'X-Turn-Id': started.turnId,
          },
        })
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
    .patch('/:agentId', async (c) => {
      const parsed = await parseAgentPatchBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)
      try {
        const agent = await service.updateAgent(
          c.req.param('agentId'),
          parsed.patch,
        )
        if (!agent) return c.json({ error: 'Unknown agent' }, 404)
        return c.json({ agent })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .get('/:agentId/sessions/:sessionId/history', async (c) => {
      const sessionId = c.req.param('sessionId')
      if (!isAgentSessionId(sessionId)) {
        return c.json({ error: 'sessionId must be "main" or a UUID' }, 400)
      }
      try {
        return c.json(
          await service.getHistory(c.req.param('agentId'), sessionId),
        )
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .post('/:agentId/chat', async (c) => {
      const agentId = c.req.param('agentId')
      const parsed = await parseChatBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)

      return startChatTurnResponse(c, service, {
        agentId,
        sessionId: MAIN_AGENT_SESSION_ID,
        parsed,
      })
    })
    .post('/:agentId/sessions/:sessionId/chat', async (c) => {
      const sessionId = parseSessionIdParam(c)
      if ('error' in sessionId) return c.json({ error: sessionId.error }, 400)
      const parsed = await parseChatBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)

      return startChatTurnResponse(c, service, {
        agentId: c.req.param('agentId'),
        sessionId: sessionId.value,
        parsed,
      })
    })
    .get('/:agentId/chat/active', (c) => {
      const agentId = c.req.param('agentId')
      const info = service.getActiveTurn(agentId, MAIN_AGENT_SESSION_ID)
      return c.json({ active: info })
    })
    .get('/:agentId/sessions/:sessionId/chat/active', (c) => {
      const sessionId = parseSessionIdParam(c)
      if ('error' in sessionId) return c.json({ error: sessionId.error }, 400)
      const info = service.getActiveTurn(
        c.req.param('agentId'),
        sessionId.value,
      )
      return c.json({ active: info })
    })
    .get('/:agentId/chat/stream', (c) => {
      const agentId = c.req.param('agentId')
      return streamExistingTurn(c, service, {
        agentId,
        sessionId: MAIN_AGENT_SESSION_ID,
      })
    })
    .get('/:agentId/sessions/:sessionId/chat/stream', (c) => {
      const sessionId = parseSessionIdParam(c)
      if ('error' in sessionId) return c.json({ error: sessionId.error }, 400)
      return streamExistingTurn(c, service, {
        agentId: c.req.param('agentId'),
        sessionId: sessionId.value,
      })
    })
    .post('/:agentId/chat/cancel', async (c) => {
      const agentId = c.req.param('agentId')
      const body = await readJsonBody(c)
      const turnId =
        'value' in body && typeof body.value.turnId === 'string'
          ? body.value.turnId.trim() || undefined
          : undefined
      const reason =
        'value' in body && typeof body.value.reason === 'string'
          ? body.value.reason
          : undefined
      const cancelled = service.cancelTurn({ agentId, turnId, reason })
      return c.json({ cancelled })
    })
    .post('/:agentId/sessions/:sessionId/chat/cancel', async (c) => {
      const sessionId = parseSessionIdParam(c)
      if ('error' in sessionId) return c.json({ error: sessionId.error }, 400)
      const body = await readJsonBody(c)
      const turnId =
        'value' in body && typeof body.value.turnId === 'string'
          ? body.value.turnId.trim() || undefined
          : undefined
      const reason =
        'value' in body && typeof body.value.reason === 'string'
          ? body.value.reason
          : undefined
      const cancelled = service.cancelTurn({
        agentId: c.req.param('agentId'),
        sessionId: sessionId.value,
        turnId,
        reason,
      })
      return c.json({ cancelled })
    })
    .get('/:agentId/queue', async (c) => {
      try {
        const queue = await service.listQueuedMessages(c.req.param('agentId'))
        return c.json({ queue })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .post('/:agentId/queue', async (c) => {
      const parsed = await parseEnqueueBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)
      try {
        const queued = await service.enqueueMessage({
          agentId: c.req.param('agentId'),
          sessionId: parsed.sessionId,
          message: parsed.message,
          attachments: parsed.attachments,
        })
        return c.json({ queued })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .post('/:agentId/sessions/:sessionId/queue', async (c) => {
      const sessionId = parseSessionIdParam(c)
      if ('error' in sessionId) return c.json({ error: sessionId.error }, 400)
      const parsed = await parseEnqueueBody(c)
      if ('error' in parsed) return c.json({ error: parsed.error }, 400)
      try {
        const queued = await service.enqueueMessage({
          agentId: c.req.param('agentId'),
          sessionId: sessionId.value,
          message: parsed.message,
          attachments: parsed.attachments,
        })
        return c.json({ queued })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
    .delete('/:agentId/queue/:messageId', async (c) => {
      try {
        const removed = await service.removeQueuedMessage({
          agentId: c.req.param('agentId'),
          messageId: c.req.param('messageId'),
        })
        if (!removed) return c.json({ error: 'Queued message not found' }, 404)
        return c.json({ removed })
      } catch (err) {
        return handleAgentRouteError(c, err)
      }
    })
}

function turnFramesToAgentEvents(
  frames: ReadableStream<TurnFrame>,
  options: { onCancel(): void | Promise<void> },
): ReadableStream<AgentStreamEvent> {
  let reader: ReadableStreamDefaultReader<TurnFrame> | undefined

  return new ReadableStream<AgentStreamEvent>({
    start() {
      reader = frames.getReader()
    },
    async pull(controller) {
      const activeReader = reader
      if (!activeReader) {
        controller.close()
        return
      }
      let result: Awaited<ReturnType<typeof activeReader.read>>
      try {
        result = await activeReader.read()
      } catch (err) {
        try {
          activeReader.releaseLock()
        } catch {}
        if (reader === activeReader) reader = undefined
        throw err
      }
      if (result?.done === false) {
        controller.enqueue(result.value.event)
      } else {
        controller.close()
        activeReader.releaseLock()
        if (reader === activeReader) reader = undefined
      }
    },
    async cancel(reason) {
      try {
        await options.onCancel()
      } finally {
        await reader?.cancel(reason).catch(() => {})
        reader = undefined
      }
    },
  })
}

type ParsedChatBody = {
  message: string
  attachments: InboundImageAttachment[]
  cwd?: string
}

async function startChatTurnResponse(
  c: Context<Env>,
  service: AgentRouteService,
  input: {
    agentId: string
    sessionId: AgentSessionId
    parsed: ParsedChatBody
  },
) {
  let started: { turnId: string; frames: ReadableStream<TurnFrame> }
  try {
    started = await service.startTurn({
      agentId: input.agentId,
      sessionId: input.sessionId,
      message: input.parsed.message,
      attachments: input.parsed.attachments,
      cwd: input.parsed.cwd,
    })
  } catch (err) {
    if (err instanceof TurnAlreadyActiveError) {
      return c.json(
        {
          error: 'Turn already active',
          turnId: err.turnId,
          attachUrl: buildChatStreamAttachUrl({
            agentId: input.agentId,
            sessionId: input.sessionId,
            turnId: err.turnId,
          }),
        },
        409,
      )
    }
    return handleAgentRouteError(c, err)
  }

  return streamTurnFrames(c, started.frames, {
    sessionId: input.sessionId,
    turnId: started.turnId,
  })
}

/**
 * Builds the stream URL clients attach to after a 409, keeping the legacy
 * main-session route while using scoped routes for sidepanel conversations.
 */
function buildChatStreamAttachUrl(input: {
  agentId: string
  sessionId: AgentSessionId
  turnId: string
}): string {
  const agentId = encodeURIComponent(input.agentId)
  const turnId = encodeURIComponent(input.turnId)
  if (input.sessionId === MAIN_AGENT_SESSION_ID) {
    return `/agents/${agentId}/chat/stream?turnId=${turnId}`
  }
  return `/agents/${agentId}/sessions/${encodeURIComponent(input.sessionId)}/chat/stream?turnId=${turnId}`
}

function streamExistingTurn(
  c: Context<Env>,
  service: AgentRouteService,
  input: {
    agentId: string
    sessionId: AgentSessionId
  },
) {
  const url = new URL(c.req.url)
  const queryTurnId = url.searchParams.get('turnId')?.trim() || undefined
  const turnId =
    queryTurnId ?? service.getActiveTurn(input.agentId, input.sessionId)?.turnId
  if (!turnId) {
    return c.json({ error: 'No active turn for this agent session' }, 404)
  }
  const lastEventId =
    c.req.header('Last-Event-ID') ??
    url.searchParams.get('lastSeq') ??
    undefined
  const lastSeq = parseLastSeq(lastEventId)
  const frames = service.attachTurn({ turnId, lastSeq })
  if (!frames) {
    return c.json({ error: 'Unknown turn' }, 404)
  }
  return streamTurnFrames(c, frames, {
    sessionId: input.sessionId,
    turnId,
  })
}

/**
 * Pipe a TurnFrame stream as Server-Sent Events. Each frame becomes:
 *
 *   id: <seq>
 *   data: <event JSON>
 *
 * followed by a final `data: [DONE]` after the upstream closes.
 * Cancelling the response (caller disconnect) detaches *this*
 * subscriber; the underlying turn keeps running in the background.
 */
function streamTurnFrames(
  c: Context<Env>,
  frames: ReadableStream<TurnFrame>,
  options: { sessionId: AgentSessionId; turnId: string },
) {
  c.header('Content-Type', 'text/event-stream')
  c.header('Cache-Control', 'no-cache')
  c.header('X-Session-Id', options.sessionId)
  c.header('X-Turn-Id', options.turnId)

  return stream(c, async (s) => {
    const reader = frames.getReader()
    const encoder = new TextEncoder()
    let completed = false
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        await s.write(
          encoder.encode(
            `id: ${value.seq}\ndata: ${JSON.stringify(value.event)}\n\n`,
          ),
        )
      }
      await s.write(encoder.encode('data: [DONE]\n\n'))
      completed = true
    } finally {
      if (completed) {
        reader.releaseLock()
      } else {
        // Caller went away mid-stream. Cancel only this subscription —
        // the registry's underlying turn keeps running.
        await reader.cancel('client disconnected').catch(() => {})
      }
    }
  })
}

function parseLastSeq(value: string | undefined): number | undefined {
  if (value == null) return undefined
  const trimmed = value.trim()
  if (trimmed === '') return undefined
  const n = Number.parseInt(trimmed, 10)
  return Number.isFinite(n) ? n : undefined
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

  // Hermes resolves its model from per-agent provider config rather
  // than from the harness catalog.
  if (
    record.adapter !== 'hermes' &&
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
interface InboundImageAttachment {
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

/**
 * Body parser for `POST /agents/:id/queue`. Mirrors `parseChatBody`'s
 * shape (message + attachments) but adds an upper bound on the
 * message text size so a runaway client can't fill the queue file
 * with multi-megabyte payloads.
 */
async function parseEnqueueBody(c: Context<Env>): Promise<
  | {
      sessionId?: AgentSessionId
      message: string
      attachments: InboundImageAttachment[]
    }
  | { error: string }
> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  const parsed = parseChatBodyRecord(body.value)
  if ('error' in parsed) return parsed
  if (parsed.message.length > AGENT_HARNESS_LIMITS.QUEUE_MESSAGE_MAX_BYTES) {
    return {
      error: `Message exceeds ${AGENT_HARNESS_LIMITS.QUEUE_MESSAGE_MAX_BYTES} bytes`,
    }
  }
  const sessionId = readOptionalTrimmedString(body.value, 'sessionId')
  if (sessionId && !isAgentSessionId(sessionId)) {
    return { error: 'sessionId must be "main" or a UUID' }
  }
  return { ...parsed, sessionId }
}

async function parseChatBody(
  c: Context<Env>,
): Promise<ParsedChatBody | { error: string }> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  return parseChatBodyRecord(body.value)
}

function parseChatBodyRecord(
  record: Record<string, unknown>,
): ParsedChatBody | { error: string } {
  const message =
    typeof record.message === 'string' ? record.message.trim() : ''
  const attachmentsRaw = Array.isArray(record.attachments)
    ? record.attachments
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
  return {
    message,
    attachments,
    cwd:
      readOptionalTrimmedString(record, 'cwd') ??
      readOptionalTrimmedString(record, 'userWorkingDir'),
  }
}

async function parseSidepanelAgentChatBody(
  c: Context<Env>,
): Promise<SidepanelAgentChatRequest | { error: string }> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  const record = body.value

  const conversationId = readOptionalTrimmedString(record, 'conversationId')
  if (!conversationId || !isUuid(conversationId)) {
    return { error: 'conversationId must be a UUID' }
  }

  const agentSessionId =
    readOptionalTrimmedString(record, 'agentSessionId') ?? conversationId
  if (!isAgentSessionId(agentSessionId)) {
    return { error: 'agentSessionId must be "main" or a UUID' }
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
    agentSessionId,
    message,
    browserContext: browserContext.value,
    selectedText,
    selectedTextSource: selectedTextSource.value,
    userSystemPrompt: readOptionalString(record, 'userSystemPrompt'),
    userWorkingDir: readOptionalTrimmedString(record, 'userWorkingDir'),
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

function isAgentSessionId(value: string): value is AgentSessionId {
  return value === MAIN_AGENT_SESSION_ID || isUuid(value)
}

function parseSessionIdParam(
  c: Context<Env>,
): { value: AgentSessionId } | { error: string } {
  const sessionId = c.req.param('sessionId')
  return isAgentSessionId(sessionId)
    ? { value: sessionId }
    : { error: 'sessionId must be "main" or a UUID' }
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
  if (err instanceof InvalidAgentUpdateError) {
    return c.json({ error: err.message }, 400)
  }
  if (err instanceof HermesProviderConfigInvalidError) {
    return c.json({ error: err.message }, 400)
  }
  if (err instanceof MessageQueueFullError) {
    return c.json({ error: err.message }, 429)
  }
  const message = err instanceof Error ? err.message : String(err)
  return c.json({ error: message }, 500)
}

async function parseAgentPatchBody(
  c: Context<Env>,
): Promise<{ patch: { name?: string; pinned?: boolean } } | { error: string }> {
  const body = await readJsonBody(c)
  if ('error' in body) return body
  const record = body.value
  const patch: { name?: string; pinned?: boolean } = {}
  if ('name' in record) {
    if (typeof record.name !== 'string') {
      return { error: 'Name must be a string' }
    }
    patch.name = record.name
  }
  if ('pinned' in record) {
    if (typeof record.pinned !== 'boolean') {
      return { error: 'Pinned must be a boolean' }
    }
    patch.pinned = record.pinned
  }
  if (Object.keys(patch).length === 0) {
    return { error: 'No editable fields supplied' }
  }
  return { patch }
}
