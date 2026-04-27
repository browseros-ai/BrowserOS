/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * HTTP routes for OpenClaw agent management.
 * Thin layer delegating to OpenClawService.
 */

import { Hono } from 'hono'
import { stream } from 'hono/streaming'
import { logger } from '../../lib/logger'
import { getMonitoringService } from '../../monitoring/service'
import type { MonitoringChatTurn } from '../../monitoring/types'
import {
  OpenClawAgentAlreadyExistsError,
  OpenClawAgentNotFoundError,
  OpenClawInvalidAgentNameError,
  OpenClawProtectedAgentError,
  OpenClawSessionNotFoundError,
} from '../services/openclaw/errors'
import { getOpenClawCliProvider } from '../services/openclaw/openclaw-cli-providers/registry'
import type { OpenClawChatContentPart } from '../services/openclaw/openclaw-http-client'
import { isUnsupportedOpenClawProviderError } from '../services/openclaw/openclaw-provider-map'
import {
  getOpenClawService,
  normalizeBrowserOSChatSessionKey,
} from '../services/openclaw/openclaw-service'
import type { QueuedItemPublic } from '../services/queue'
import { getOutboundQueueService } from '../services/queue'

/**
 * Inbound attachment shapes the chat route accepts. Images travel as
 * data: URLs (the gateway is on 127.0.0.1 so we don't pay public-network
 * cost for the base64 overhead). Files arrive with their text already
 * extracted on the client — we just inline them as a fenced text part on
 * the user message.
 */
type ImageAttachment = {
  kind: 'image'
  mediaType: string
  dataUrl: string
  name?: string
}
type FileAttachment = {
  kind: 'file'
  mediaType: string
  name: string
  text: string
}
type ChatAttachment = ImageAttachment | FileAttachment

const MAX_ATTACHMENTS = 10
const MAX_IMAGE_BYTES = 5 * 1024 * 1024 // 5 MB after compression
// data: URLs encode bytes as base64 (~4/3 inflation) plus a small media-type
// prefix; cap the encoded string against that, not 2× the byte budget.
const MAX_IMAGE_DATA_URL_LENGTH = Math.ceil(MAX_IMAGE_BYTES * (4 / 3)) + 100
const MAX_FILE_TEXT_BYTES = 1 * 1024 * 1024 // 1 MB extracted text
const ALLOWED_IMAGE_MEDIA_TYPES = new Set([
  'image/png',
  'image/jpeg',
  'image/jpg',
  'image/webp',
  'image/gif',
])
const ALLOWED_FILE_MEDIA_TYPE_PREFIXES = ['text/', 'application/json']

function validateChatAttachments(input: unknown): {
  attachments: ChatAttachment[] | null
  error: string | null
} {
  if (input === undefined || input === null) {
    return { attachments: null, error: null }
  }
  if (!Array.isArray(input)) {
    return { attachments: null, error: 'attachments must be an array' }
  }
  if (input.length > MAX_ATTACHMENTS) {
    return {
      attachments: null,
      error: `at most ${MAX_ATTACHMENTS} attachments are allowed per message`,
    }
  }

  const result: ChatAttachment[] = []
  for (const raw of input) {
    if (!raw || typeof raw !== 'object') {
      return { attachments: null, error: 'invalid attachment entry' }
    }
    const entry = raw as Record<string, unknown>
    if (entry.kind === 'image') {
      const mediaType =
        typeof entry.mediaType === 'string' ? entry.mediaType : ''
      const dataUrl = typeof entry.dataUrl === 'string' ? entry.dataUrl : ''
      if (!ALLOWED_IMAGE_MEDIA_TYPES.has(mediaType)) {
        return {
          attachments: null,
          error: `unsupported image type: ${mediaType || 'unknown'}`,
        }
      }
      if (!dataUrl.startsWith('data:')) {
        return {
          attachments: null,
          error: 'image attachment must include a data: URL',
        }
      }
      if (dataUrl.length > MAX_IMAGE_DATA_URL_LENGTH) {
        return {
          attachments: null,
          error: `image exceeds ${MAX_IMAGE_BYTES} bytes`,
        }
      }
      result.push({
        kind: 'image',
        mediaType,
        dataUrl,
        name: typeof entry.name === 'string' ? entry.name : undefined,
      })
      continue
    }
    if (entry.kind === 'file') {
      const mediaType =
        typeof entry.mediaType === 'string' ? entry.mediaType : ''
      const name = typeof entry.name === 'string' ? entry.name : ''
      const text = typeof entry.text === 'string' ? entry.text : ''
      const allowed = ALLOWED_FILE_MEDIA_TYPE_PREFIXES.some((prefix) =>
        mediaType.startsWith(prefix),
      )
      if (!allowed) {
        return {
          attachments: null,
          error: `unsupported file type: ${mediaType || 'unknown'}`,
        }
      }
      if (!name) {
        return {
          attachments: null,
          error: 'file attachment must include a name',
        }
      }
      if (text.length > MAX_FILE_TEXT_BYTES) {
        return {
          attachments: null,
          error: `file "${name}" exceeds ${MAX_FILE_TEXT_BYTES} bytes`,
        }
      }
      result.push({ kind: 'file', mediaType, name, text })
      continue
    }
    return {
      attachments: null,
      error: 'attachment kind must be "image" or "file"',
    }
  }
  return { attachments: result, error: null }
}

function buildMessagePartsFromAttachments(
  message: string,
  attachments: ChatAttachment[],
): { text: string; parts: OpenClawChatContentPart[] | undefined } {
  const images = attachments.filter(
    (a): a is ImageAttachment => a.kind === 'image',
  )
  const files = attachments.filter(
    (a): a is FileAttachment => a.kind === 'file',
  )

  const fileBlocks = files
    .map(
      (f) => `<attachment name="${f.name}" mediaType="${f.mediaType}">
${f.text}
</attachment>`,
    )
    .join('\n\n')
  const text = fileBlocks ? `${message}\n\n${fileBlocks}`.trim() : message

  if (images.length === 0) {
    return { text, parts: undefined }
  }

  const parts: OpenClawChatContentPart[] = [{ type: 'text', text }]
  for (const image of images) {
    parts.push({ type: 'image_url', image_url: { url: image.dataUrl } })
  }
  return { text, parts }
}

function getCreateAgentValidationError(body: { name?: string }): string | null {
  if (!body.name?.trim()) {
    return 'Name is required'
  }
  return null
}

function parsePositiveIntQuery(
  value: string | undefined,
  fallback: number,
): number {
  if (value === undefined) return fallback
  const parsed = Number(value)
  if (!Number.isFinite(parsed)) return fallback
  return Math.max(1, Math.trunc(parsed))
}

export function createOpenClawRoutes() {
  return new Hono()
    .get('/status', async (c) => {
      const status = await getOpenClawService().getStatus()
      return c.json(status)
    })

    .get('/providers/:providerId/auth-status', async (c) => {
      const { providerId } = c.req.param()
      const provider = getOpenClawCliProvider(providerId)
      if (!provider) {
        return c.json({ error: `Unknown CLI provider: ${providerId}` }, 404)
      }
      try {
        const status =
          await getOpenClawService().getCliProviderAuthStatus(provider)
        return c.json(status)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.warn('CLI provider auth-status failed', {
          providerId,
          error: message,
        })
        return c.json(
          { installed: false, loggedIn: false, error: message },
          500,
        )
      }
    })

    .post('/setup', async (c) => {
      const body = await c.req.json<{
        providerType?: string
        providerName?: string
        baseUrl?: string
        apiKey?: string
        modelId?: string
      }>()

      try {
        logger.info('OpenClaw setup requested', {
          providerType: body.providerType,
          providerName: body.providerName,
          hasBaseUrl: !!body.baseUrl,
          hasModel: !!body.modelId,
          hasApiKey: !!body.apiKey,
        })
        const logs: string[] = []
        await getOpenClawService().setup(body, (msg) => logs.push(msg))

        const agents = await getOpenClawService().listAgents()
        return c.json(
          {
            status: 'running',
            port: getOpenClawService().getPort(),
            agents: agents.map((a) => ({
              agentId: a.agentId,
              name: a.name,
              status: 'running',
            })),
            logs,
          },
          201,
        )
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.error('OpenClaw setup failed', {
          error: message,
          providerType: body.providerType,
          providerName: body.providerName,
        })
        if (isUnsupportedOpenClawProviderError(err)) {
          return c.json({ error: err.message }, 400)
        }
        if (message.includes('VM runtime is not available')) {
          return c.json({ error: message }, 503)
        }
        return c.json({ error: message }, 500)
      }
    })

    .post('/start', async (c) => {
      try {
        logger.info('OpenClaw start requested')
        await getOpenClawService().start()
        return c.json({ status: 'running' })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.error('OpenClaw start failed', { error: message })
        return c.json({ error: message }, 500)
      }
    })

    .post('/stop', async (c) => {
      try {
        logger.info('OpenClaw stop requested')
        await getOpenClawService().stop()
        return c.json({ status: 'stopped' })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.error('OpenClaw stop failed', { error: message })
        return c.json({ error: message }, 500)
      }
    })

    .post('/restart', async (c) => {
      try {
        logger.info('OpenClaw restart requested')
        await getOpenClawService().restart()
        return c.json({ status: 'running' })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.error('OpenClaw restart failed', { error: message })
        return c.json({ error: message }, 500)
      }
    })

    .post('/reconnect', async (c) => {
      try {
        logger.info('OpenClaw reconnect requested')
        await getOpenClawService().reconnectControlPlane()
        return c.json({ status: 'connected' })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.error('OpenClaw reconnect failed', { error: message })
        return c.json({ error: message }, 500)
      }
    })

    .get('/agents', async (c) => {
      try {
        const agents = await getOpenClawService().listAgents()
        return c.json({ agents })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .post('/agents', async (c) => {
      const body = await c.req.json<{
        name: string
        providerType?: string
        providerName?: string
        baseUrl?: string
        apiKey?: string
        modelId?: string
      }>()
      const validationError = getCreateAgentValidationError(body)
      if (validationError) {
        return c.json({ error: validationError }, 400)
      }

      try {
        const agent = await getOpenClawService().createAgent({
          name: body.name.trim(),
          providerType: body.providerType,
          providerName: body.providerName,
          baseUrl: body.baseUrl,
          apiKey: body.apiKey,
          modelId: body.modelId,
        })
        return c.json({ agent }, 201)
      } catch (err) {
        if (err instanceof OpenClawAgentAlreadyExistsError) {
          return c.json({ error: err.message }, 409)
        }
        if (err instanceof OpenClawInvalidAgentNameError) {
          return c.json({ error: err.message }, 400)
        }
        if (isUnsupportedOpenClawProviderError(err)) {
          return c.json({ error: err.message }, 400)
        }
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .delete('/agents/:id', async (c) => {
      const { id } = c.req.param()

      try {
        await getOpenClawService().removeAgent(id)
        return c.json({ success: true })
      } catch (err) {
        if (err instanceof OpenClawAgentNotFoundError) {
          return c.json({ error: err.message }, 404)
        }
        if (err instanceof OpenClawProtectedAgentError) {
          return c.json({ error: err.message }, 400)
        }
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .get('/agents/:id/sessions', async (c) => {
      const { id } = c.req.param()
      const limit = parsePositiveIntQuery(c.req.query('limit'), 20)

      try {
        const sessions = await getOpenClawService().listSessions(id)
        return c.json({
          agentId: id,
          sessions: sessions.slice(0, Math.min(limit, 100)),
        })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .get('/agents/:id/session', async (c) => {
      const { id } = c.req.param()

      try {
        const session = await getOpenClawService().resolveAgentSession(id)
        return c.json(session)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .get('/agents/:id/history', async (c) => {
      const { id } = c.req.param()
      const limit = parsePositiveIntQuery(c.req.query('limit'), 50)

      try {
        const page = await getOpenClawService().getAgentHistoryPage(id, {
          sessionKey: c.req.query('sessionKey'),
          cursor: c.req.query('cursor'),
          limit,
        })
        return c.json(page)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .get('/dashboard', (c) => {
      try {
        const dashboard = getOpenClawService().getDashboard()
        return c.json(dashboard)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .get('/dashboard/stream', (c) => {
      c.header('Content-Type', 'text/event-stream')
      c.header('Cache-Control', 'no-cache')
      c.header('Connection', 'keep-alive')

      return stream(c, async (s) => {
        const encoder = new TextEncoder()

        // Send initial snapshot
        try {
          const dashboard = getOpenClawService().getDashboard()
          await s.write(
            encoder.encode(
              `event: snapshot\ndata: ${JSON.stringify(dashboard)}\n\n`,
            ),
          )
        } catch {}

        // Subscribe to live status changes
        const unsubscribe = getOpenClawService().onAgentStatusChange(
          (agentId, entry) => {
            const event = {
              agentId,
              status: entry.status,
              currentTool: entry.currentTool,
              error: entry.error,
              timestamp: entry.lastEventAt,
            }
            s.write(
              encoder.encode(
                `event: status\ndata: ${JSON.stringify(event)}\n\n`,
              ),
            ).catch(() => {})
          },
        )

        // Heartbeat every 15s to keep connection alive
        const heartbeat = setInterval(() => {
          s.write(
            encoder.encode(
              `event: heartbeat\ndata: ${JSON.stringify({ ts: Date.now() })}\n\n`,
            ),
          ).catch(() => {})
        }, 15_000)

        // Wait until client disconnects
        try {
          await new Promise<void>((resolve) => {
            s.onAbort(() => resolve())
          })
        } finally {
          unsubscribe()
          clearInterval(heartbeat)
        }
      })
    })
    .post('/agents/:id/chat', async (c) => {
      const { id } = c.req.param()
      const body = await c.req.json<{
        message: string
        sessionKey?: string
        history?: MonitoringChatTurn[]
        attachments?: unknown
      }>()

      const trimmedMessage = body.message?.trim() ?? ''
      const attachmentValidation = validateChatAttachments(body.attachments)
      if (attachmentValidation.error) {
        return c.json({ error: attachmentValidation.error }, 400)
      }
      const attachments = attachmentValidation.attachments ?? []
      // Either a non-empty text body or at least one attachment is required.
      if (!trimmedMessage && attachments.length === 0) {
        return c.json({ error: 'Message is required' }, 400)
      }

      const sessionKey = normalizeBrowserOSChatSessionKey(
        id,
        body.sessionKey ?? crypto.randomUUID(),
      )
      const history = Array.isArray(body.history)
        ? body.history.filter((entry): entry is MonitoringChatTurn =>
            Boolean(
              entry &&
                (entry.role === 'user' || entry.role === 'assistant') &&
                typeof entry.content === 'string',
            ),
          )
        : []

      // Replace the immediate 409 with a bounded wait so back-to-back user
      // sends or a cron / hook turn that's still finishing don't reject the
      // user-chat outright. The client-side outbound queue (Feature 2) keeps
      // the per-agent send rate at 1, so this only kicks in for cross-source
      // contention.
      try {
        await getMonitoringService().waitForSessionFree(id, {
          timeoutMs: 30_000,
        })
      } catch (err) {
        return c.json(
          {
            error:
              err instanceof Error
                ? err.message
                : 'Agent is busy. Try again shortly.',
          },
          503,
        )
      }

      const { text: composedMessage, parts: messageParts } =
        buildMessagePartsFromAttachments(trimmedMessage, attachments)

      const monitoringContext = await getMonitoringService().startSession({
        agentId: id,
        sessionKey,
        originalPrompt: composedMessage,
        chatHistory: history,
      })

      try {
        const eventStream = await getOpenClawService().chatStream(
          id,
          sessionKey,
          composedMessage,
          history,
          { messageParts },
        )

        c.header('Content-Type', 'text/event-stream')
        c.header('Cache-Control', 'no-cache')
        c.header('X-Session-Key', sessionKey)

        return stream(c, async (s) => {
          const reader = eventStream.getReader()
          const encoder = new TextEncoder()
          let finalAssistantMessage: string | undefined
          let status: 'completed' | 'failed' | 'aborted' | 'incomplete' =
            'incomplete'
          let finalError: string | undefined
          try {
            while (true) {
              const { done, value } = await reader.read()
              if (done) break
              if (
                value.type === 'done' &&
                typeof value.data.text === 'string' &&
                value.data.text.trim()
              ) {
                finalAssistantMessage = value.data.text
                status = 'completed'
              }
              if (value.type === 'error') {
                finalError =
                  (typeof value.data.message === 'string'
                    ? value.data.message
                    : typeof value.data.error === 'string'
                      ? value.data.error
                      : undefined) ?? 'Unknown chat stream error'
                status = 'failed'
              }
              await s.write(
                encoder.encode(`data: ${JSON.stringify(value)}\n\n`),
              )
            }
            await s.write(encoder.encode('data: [DONE]\n\n'))
          } catch (error) {
            if (c.req.raw.signal.aborted) {
              status = 'aborted'
            } else {
              status = 'failed'
              finalError =
                error instanceof Error ? error.message : String(error)
            }
            throw error
          } finally {
            await reader.cancel()
            await getMonitoringService().finalizeSession({
              monitoringSessionId: monitoringContext.monitoringSessionId,
              agentId: id,
              sessionKey,
              status,
              finalAssistantMessage,
              error: finalError,
            })
          }
        })
      } catch (err) {
        await getMonitoringService().finalizeSession({
          monitoringSessionId: monitoringContext.monitoringSessionId,
          agentId: id,
          sessionKey,
          status: c.req.raw.signal.aborted ? 'aborted' : 'failed',
          error: err instanceof Error ? err.message : String(err),
        })
        if (isUnsupportedOpenClawProviderError(err)) {
          return c.json({ error: err.message }, 400)
        }
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .post('/agents/:id/queue', async (c) => {
      const { id } = c.req.param()
      const body = await c.req.json<{
        message: string
        sessionKey?: string
        history?: MonitoringChatTurn[]
        attachments?: unknown
        // Optional client-provided id — when set, the queue uses it as
        // the canonical item id so the browser's optimistic row and the
        // SSE snapshot reconcile on the same key.
        id?: string
      }>()
      const trimmedMessage = body.message?.trim() ?? ''
      const attachmentValidation = validateChatAttachments(body.attachments)
      if (attachmentValidation.error) {
        return c.json({ error: attachmentValidation.error }, 400)
      }
      const attachments = attachmentValidation.attachments ?? []
      if (!trimmedMessage && attachments.length === 0) {
        return c.json({ error: 'Message is required' }, 400)
      }

      const sessionKey = body.sessionKey
        ? normalizeBrowserOSChatSessionKey(id, body.sessionKey)
        : undefined
      const history = Array.isArray(body.history)
        ? body.history.filter((entry): entry is MonitoringChatTurn =>
            Boolean(
              entry &&
                (entry.role === 'user' || entry.role === 'assistant') &&
                typeof entry.content === 'string',
            ),
          )
        : []

      const { text: composedMessage, parts: messageParts } =
        buildMessagePartsFromAttachments(trimmedMessage, attachments)

      const item = getOutboundQueueService().enqueue({
        agentId: id,
        id: typeof body.id === 'string' && body.id ? body.id : undefined,
        message: composedMessage,
        messageParts,
        sessionKey,
        history,
        attachmentsPreview: attachments.map((a) => ({
          kind: a.kind,
          mediaType: a.mediaType,
          name: 'name' in a ? a.name : undefined,
        })),
      })
      return c.json({ id: item.id }, 202)
    })

    .delete('/agents/:id/queue/:itemId', (c) => {
      const { id, itemId } = c.req.param()
      const result = getOutboundQueueService().cancel(id, itemId)
      if (!result.ok) {
        const code = result.reason === 'dispatching' ? 409 : 404
        const message =
          result.reason === 'dispatching'
            ? 'Item is already dispatching'
            : 'Item not found'
        return c.json({ error: message }, code)
      }
      return c.json({ ok: true })
    })

    .post('/agents/:id/queue/:itemId/retry', (c) => {
      const { id, itemId } = c.req.param()
      const result = getOutboundQueueService().retry(id, itemId)
      if (!result.ok) {
        return c.json({ error: 'Item not found or not failed' }, 404)
      }
      return c.json({ ok: true })
    })

    .get('/agents/:id/queue/stream', (c) => {
      const { id } = c.req.param()
      c.header('Content-Type', 'text/event-stream')
      c.header('Cache-Control', 'no-cache')
      return stream(c, async (s) => {
        const encoder = new TextEncoder()
        const sendSnapshot = (items: QueuedItemPublic[]) => {
          void s.write(encoder.encode(`data: ${JSON.stringify({ items })}\n\n`))
        }
        const unsubscribe = getOutboundQueueService().subscribe(
          id,
          sendSnapshot,
        )
        const heartbeat = setInterval(() => {
          void s.write(encoder.encode(': keep-alive\n\n'))
        }, 15_000)
        try {
          await new Promise<void>((resolve) => {
            s.onAbort(() => resolve())
          })
        } finally {
          clearInterval(heartbeat)
          unsubscribe()
        }
      })
    })

    .get('/session/:key/history', async (c) => {
      const key = c.req.param('key')
      const limitRaw = c.req.query('limit')
      const cursor = c.req.query('cursor')
      const limitParsed =
        limitRaw !== undefined ? Number.parseInt(limitRaw, 10) : Number.NaN
      const limit = Number.isFinite(limitParsed) ? limitParsed : undefined
      const wantsStream = (c.req.header('accept') ?? '').includes(
        'text/event-stream',
      )

      try {
        if (!wantsStream) {
          const history = await getOpenClawService().getSessionHistory(key, {
            limit,
            cursor,
          })
          return c.json(history)
        }

        const eventStream = await getOpenClawService().streamSessionHistory(
          key,
          { limit, cursor, signal: c.req.raw.signal },
        )

        c.header('Content-Type', 'text/event-stream')
        c.header('Cache-Control', 'no-cache')
        c.header('X-Session-Key', key)

        return stream(c, async (s) => {
          const reader = eventStream.getReader()
          const encoder = new TextEncoder()
          try {
            while (true) {
              const { done, value } = await reader.read()
              if (done) break
              await s.write(
                encoder.encode(
                  `event: ${value.type}\ndata: ${JSON.stringify(value.data)}\n\n`,
                ),
              )
            }
          } finally {
            await reader.cancel()
          }
        })
      } catch (err) {
        if (err instanceof OpenClawSessionNotFoundError) {
          return c.json({ error: err.message }, 404)
        }
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .get('/logs', async (c) => {
      try {
        const logs = await getOpenClawService().getLogs()
        return c.json({ logs })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .post('/providers', async (c) => {
      const body = await c.req.json<{
        providerType: string
        apiKey: string
        providerName?: string
        baseUrl?: string
        modelId?: string
      }>()

      if (!body.providerType || !body.apiKey) {
        return c.json({ error: 'providerType and apiKey are required' }, 400)
      }

      try {
        const result = await getOpenClawService().updateProviderKeys(body)
        return c.json({
          status: result.restarted ? 'restarting' : 'updated',
          message: result.restarted
            ? 'Provider updated, restarting gateway'
            : 'Provider updated without a restart',
        })
      } catch (err) {
        if (isUnsupportedOpenClawProviderError(err)) {
          return c.json({ error: err.message }, 400)
        }
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })
}
