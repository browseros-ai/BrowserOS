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
import {
  OpenClawAgentAlreadyExistsError,
  OpenClawAgentNotFoundError,
  OpenClawInvalidAgentNameError,
  OpenClawProtectedAgentError,
  OpenClawSessionNotFoundError,
} from '../services/openclaw/errors'
import { getOpenClawCliProvider } from '../services/openclaw/openclaw-cli-providers/registry'
import { isUnsupportedOpenClawProviderError } from '../services/openclaw/openclaw-provider-map'
import { getOpenClawService } from '../services/openclaw/openclaw-service'

function getCreateAgentValidationError(body: { name?: string }): string | null {
  if (!body.name?.trim()) {
    return 'Name is required'
  }
  return null
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
        supportsImages?: boolean
      }>()

      try {
        logger.info('OpenClaw setup requested', {
          providerType: body.providerType,
          providerName: body.providerName,
          hasBaseUrl: !!body.baseUrl,
          hasModel: !!body.modelId,
          hasApiKey: !!body.apiKey,
          supportsImages: !!body.supportsImages,
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
        supportsImages?: boolean
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
          supportsImages: body.supportsImages,
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
