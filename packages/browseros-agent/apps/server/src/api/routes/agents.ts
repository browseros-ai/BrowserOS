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
import type { OpenclawGatewayAccessor } from '../../lib/agents/acpx-runtime'
import type {
  ActiveTurnInfo,
  TurnFrame,
} from '../../lib/agents/active-turn-registry'
import { AdapterHealthChecker } from '../../lib/agents/adapter-health'
import {
  AGENT_ADAPTER_CATALOG,
  isAgentAdapter,
  isSupportedAgentModel,
  isSupportedReasoningEffort,
} from '../../lib/agents/agent-catalog'
import type {
  AgentAdapter,
  AgentDefinition,
} from '../../lib/agents/agent-types'
import type { AgentHistoryPage, AgentStreamEvent } from '../../lib/agents/types'
import {
  type AgentDefinitionWithActivity,
  AgentHarnessService,
  type GatewayStatusSnapshot,
  InvalidAgentUpdateError,
  MessageQueueFullError,
  type OpenClawProvisioner,
  OpenClawProvisionerUnavailableError,
  type ProducedFileEntry,
  type ProducedFilesRailGroup,
  type QueuedMessage,
  TurnAlreadyActiveError,
  UnknownAgentError,
} from '../services/agents/agent-harness-service'
import type { FilePreview } from '../services/openclaw/file-preview'
import type { OpenClawGatewayChatClient } from '../services/openclaw/openclaw-gateway-chat-client'
import type { Env } from '../types'
import { resolveBrowserContextPageIds } from '../utils/resolve-browser-context-page-ids'

type AgentRouteService = {
  listAgents(): Promise<AgentDefinition[]>
  listAgentsWithActivity(): Promise<AgentDefinitionWithActivity[]>
  getGatewayStatus(): Promise<GatewayStatusSnapshot | null>
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
  getHistory(agentId: string): Promise<AgentHistoryPage>
  startTurn(input: {
    agentId: string
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    cwd?: string
  }): Promise<{ turnId: string; frames: ReadableStream<TurnFrame> }>
  attachTurn(input: {
    turnId: string
    lastSeq?: number
  }): ReadableStream<TurnFrame> | null
  getActiveTurn(agentId: string, sessionId?: 'main'): ActiveTurnInfo | null
  cancelTurn(input: {
    agentId: string
    turnId?: string
    reason?: string
  }): boolean
  enqueueMessage(input: {
    agentId: string
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
  }): Promise<QueuedMessage>
  removeQueuedMessage(input: {
    agentId: string
    messageId: string
  }): Promise<boolean>
  listQueuedMessages(agentId: string): Promise<QueuedMessage[]>

  // Files API — Phase 3 of TKT-762.
  listAgentFiles(
    agentId: string,
    options?: { limit?: number },
  ): Promise<ProducedFilesRailGroup[]>
  listAgentFilesForTurn(
    agentId: string,
    turnId: string,
  ): Promise<ProducedFileEntry[]>
  previewProducedFile(fileId: string): Promise<FilePreview | null>
  resolveProducedFileForDownload(fileId: string): Promise<{
    absolutePath: string
    fileName: string
    mimeType: string
    size: number
  } | null>
}

type AgentRouteDeps = {
  service?: AgentRouteService
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
  /** Optional override; defaults to a fresh in-memory checker. */
  adapterHealth?: AdapterHealthChecker
}

type SidepanelAgentChatRequest = {
  conversationId: string
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
  // One checker per route mount. Cached probes refresh every 5min;
  // tests can swap in an alternate via deps if needed.
  const adapterHealth = deps.adapterHealth ?? new AdapterHealthChecker()

  return (
    new Hono<Env>()
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
        // Single round-trip the agents page consumes: enriched agents
        // (status + lastUsedAt) plus the gateway lifecycle snapshot the
        // GatewayStatusBar / GatewayStateCards / ControlPlaneAlert used
        // to fetch from `/claw/status`. Lets the page poll one endpoint.
        const [agents, gateway] = await Promise.all([
          service.listAgentsWithActivity(),
          service.getGatewayStatus(),
        ])
        return c.json({ agents, gateway })
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
              message,
              cwd: parsed.userWorkingDir,
            })
          } catch (err) {
            if (err instanceof TurnAlreadyActiveError) {
              return c.json(
                {
                  error: 'Turn already active',
                  turnId: err.turnId,
                  attachUrl: `/agents/${agent.id}/chat/stream?turnId=${err.turnId}`,
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
              'X-Session-Id': 'main',
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

        let started: { turnId: string; frames: ReadableStream<TurnFrame> }
        try {
          started = await service.startTurn({
            agentId,
            message: parsed.message,
            attachments: parsed.attachments,
            cwd: parsed.cwd,
          })
        } catch (err) {
          if (err instanceof TurnAlreadyActiveError) {
            // Caller can attach via GET /chat/stream?turnId=… instead.
            return c.json(
              {
                error: 'Turn already active',
                turnId: err.turnId,
                attachUrl: `/agents/${agentId}/chat/stream?turnId=${err.turnId}`,
              },
              409,
            )
          }
          return handleAgentRouteError(c, err)
        }

        return streamTurnFrames(c, started.frames, {
          turnId: started.turnId,
        })
      })
      .get('/:agentId/chat/active', (c) => {
        const agentId = c.req.param('agentId')
        const info = service.getActiveTurn(agentId, 'main')
        return c.json({ active: info })
      })
      .get('/:agentId/chat/stream', (c) => {
        const agentId = c.req.param('agentId')
        const url = new URL(c.req.url)
        const queryTurnId = url.searchParams.get('turnId')?.trim() || undefined
        const turnId =
          queryTurnId ?? service.getActiveTurn(agentId, 'main')?.turnId
        if (!turnId) {
          return c.json({ error: 'No active turn for this agent' }, 404)
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
        return streamTurnFrames(c, frames, { turnId })
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
          if (!removed)
            return c.json({ error: 'Queued message not found' }, 404)
          return c.json({ removed })
        } catch (err) {
          return handleAgentRouteError(c, err)
        }
      })

      // ── Files (TKT-762) ────────────────────────────────────────────
      //
      // V1 surfaces files OpenClaw agents produce inside their workspace
      // dir (`~/.browseros/vm/openclaw/.openclaw/workspace[-<name>]/`)
      // as outputs, attributed back to the chat turn that produced them
      // by the per-turn workspace diff in
      // `agent-harness-service.runDetachedTurn`. Adapter-gated to
      // openclaw on the service side; for claude / codex these endpoints
      // simply return empty lists.
      //
      // The file-id-scoped endpoints (`/files/:fileId/{preview,download}`)
      // accept an opaque `fileId` and resolve the on-disk path
      // server-side, so the client never sees a raw path and traversal
      // is impossible by construction.

      .get('/:agentId/files', async (c) => {
        try {
          const groups = await service.listAgentFiles(
            c.req.param('agentId'),
            parseAgentFilesLimit(c.req.query('limit')),
          )
          return c.json({ groups })
        } catch (err) {
          return handleAgentRouteError(c, err)
        }
      })
      .get('/:agentId/files/turn/:turnId', async (c) => {
        try {
          const files = await service.listAgentFilesForTurn(
            c.req.param('agentId'),
            c.req.param('turnId'),
          )
          return c.json({ files })
        } catch (err) {
          return handleAgentRouteError(c, err)
        }
      })
      .get('/files/:fileId/preview', async (c) => {
        try {
          const preview = await service.previewProducedFile(
            c.req.param('fileId'),
          )
          if (!preview || preview.kind === 'missing') {
            return c.json({ error: 'File not found' }, 404)
          }
          return c.json(preview)
        } catch (err) {
          return handleAgentRouteError(c, err)
        }
      })
      .get('/files/:fileId/download', async (c) => {
        try {
          const resolved = await service.resolveProducedFileForDownload(
            c.req.param('fileId'),
          )
          if (!resolved) return c.json({ error: 'File not found' }, 404)

          // Stream raw bytes via Bun's lazy file handle. Sets
          // Content-Disposition so browsers save instead of preview.
          const file = Bun.file(resolved.absolutePath)
          return new Response(file.stream(), {
            headers: {
              'Content-Type': resolved.mimeType,
              'Content-Length': String(resolved.size),
              'Content-Disposition': `attachment; ${encodeRfc6266Filename(resolved.fileName)}`,
              'Cache-Control': 'no-store',
            },
          })
        } catch (err) {
          return handleAgentRouteError(c, err)
        }
      })
  )
}

/** Hard cap on `?limit=` for /agents/:id/files — guards against
 *  a caller-supplied huge value forcing a per-agent table scan. */
const MAX_FILES_LIMIT = 500

/**
 * Parse + clamp the `limit` query for /agents/:id/files. Returns
 * `undefined` when the param is absent or unparseable so the
 * service falls back to its own default.
 */
function parseAgentFilesLimit(
  raw: string | undefined,
): { limit: number } | undefined {
  if (!raw) return undefined
  const parsed = Number.parseInt(raw, 10)
  if (!Number.isFinite(parsed)) return undefined
  return { limit: Math.min(Math.max(1, parsed), MAX_FILES_LIMIT) }
}

/**
 * RFC 6266 / RFC 5987 filename attributes for `Content-Disposition`.
 * Returns the `filename="..."` attribute (always) plus a
 * percent-encoded `filename*=UTF-8''…` attribute when the name
 * contains non-ASCII characters, so browsers download with the
 * original name even on stricter HTTP clients.
 */
function encodeRfc6266Filename(filename: string): string {
  // Strip CRLFs and quotes (header injection guard).
  const safe = filename.replace(/["\r\n]/g, '_')
  // Detect non-ASCII; emit the RFC 5987 fallback attribute when
  // present. `encodeURIComponent` is the standard browser-safe
  // percent-encoder for this purpose.
  const hasNonAscii = /[^ -~]/.test(safe)
  if (!hasNonAscii) return `filename="${safe}"`
  return `filename="${safe}"; filename*=UTF-8''${encodeURIComponent(safe)}`
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
  options: { turnId: string },
) {
  c.header('Content-Type', 'text/event-stream')
  c.header('Cache-Control', 'no-cache')
  c.header('X-Session-Id', 'main')
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

/**
 * Body parser for `POST /agents/:id/queue`. Mirrors `parseChatBody`'s
 * shape (message + attachments) but adds an upper bound on the
 * message text size so a runaway client can't fill the queue file
 * with multi-megabyte payloads.
 */
async function parseEnqueueBody(
  c: Context<Env>,
): Promise<
  { message: string; attachments: InboundImageAttachment[] } | { error: string }
> {
  const parsed = await parseChatBody(c)
  if ('error' in parsed) return parsed
  if (parsed.message.length > AGENT_HARNESS_LIMITS.QUEUE_MESSAGE_MAX_BYTES) {
    return {
      error: `Message exceeds ${AGENT_HARNESS_LIMITS.QUEUE_MESSAGE_MAX_BYTES} bytes`,
    }
  }
  return parsed
}

async function parseChatBody(
  c: Context<Env>,
): Promise<
  | { message: string; attachments: InboundImageAttachment[]; cwd?: string }
  | { error: string }
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
  return {
    message,
    attachments,
    cwd:
      readOptionalTrimmedString(body.value, 'cwd') ??
      readOptionalTrimmedString(body.value, 'userWorkingDir'),
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

  const message = readOptionalTrimmedString(record, 'message')
  if (!message) return { error: 'Message is required' }

  const browserContext = parseBrowserContext(record.browserContext)
  if ('error' in browserContext) return browserContext

  const selectedText = readOptionalString(record, 'selectedText')
  const selectedTextSource = parseSelectedTextSource(record.selectedTextSource)
  if ('error' in selectedTextSource) return selectedTextSource

  return {
    conversationId,
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
  if (err instanceof MessageQueueFullError) {
    return c.json({ error: err.message }, 429)
  }
  if (err instanceof OpenClawProvisionerUnavailableError) {
    return c.json({ error: err.message }, 503)
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
