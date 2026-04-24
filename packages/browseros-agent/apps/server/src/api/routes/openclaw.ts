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
import { isUnsupportedOpenClawProviderError } from '../services/openclaw/openclaw-provider-map'
import {
  getOpenClawService,
  normalizeBrowserOSChatSessionKey,
} from '../services/openclaw/openclaw-service'

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

    .post('/agents/:id/chat', async (c) => {
      const { id } = c.req.param()
      const body = await c.req.json<{
        message: string
        sessionKey?: string
        history?: MonitoringChatTurn[]
      }>()

      if (!body.message?.trim()) {
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
      if (getMonitoringService().getActiveSessionId(id)) {
        return c.json(
          {
            error:
              'A monitored chat session is already active for this agent. Wait for it to finish before starting another.',
          },
          409,
        )
      }
      const monitoringContext = await getMonitoringService().startSession({
        agentId: id,
        sessionKey,
        originalPrompt: body.message.trim(),
        chatHistory: history,
      })

      try {
        const eventStream = await getOpenClawService().chatStream(
          id,
          sessionKey,
          body.message,
          history,
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
