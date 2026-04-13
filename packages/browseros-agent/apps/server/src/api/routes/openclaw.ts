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
import { getOpenClawService } from '../../services/openclaw/openclaw-service'

export function createOpenClawRoutes() {
  return new Hono()
    .get('/status', async (c) => {
      const status = await getOpenClawService().getStatus()
      return c.json(status)
    })

    .post('/setup', async (c) => {
      const body = await c.req.json<{
        providerType?: string
        apiKey?: string
        modelId?: string
      }>()

      try {
        const logs: string[] = []
        await getOpenClawService().setup(body, (msg) => logs.push(msg))

        const agents = await getOpenClawService().listAgents()
        return c.json(
          {
            status: 'running',
            port: 18789,
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
        if (message.includes('Podman is not available')) {
          return c.json({ error: message }, 503)
        }
        return c.json({ error: message }, 500)
      }
    })

    .post('/start', async (c) => {
      try {
        await getOpenClawService().start()
        return c.json({ status: 'running' })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .post('/stop', async (c) => {
      try {
        await getOpenClawService().stop()
        return c.json({ status: 'stopped' })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })

    .post('/restart', async (c) => {
      try {
        await getOpenClawService().restart()
        return c.json({ status: 'running' })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
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
        apiKey?: string
        modelId?: string
      }>()
      const name = body.name?.trim()

      if (!name) {
        return c.json({ error: 'Name is required' }, 400)
      }

      try {
        const agent = await getOpenClawService().createAgent({
          name,
          providerType: body.providerType,
          apiKey: body.apiKey,
          modelId: body.modelId,
        })
        return c.json({ agent }, 201)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        if (message.includes('already exists')) {
          return c.json({ error: message }, 409)
        }
        if (message.includes('must start with')) {
          return c.json({ error: message }, 400)
        }
        return c.json({ error: message }, 500)
      }
    })

    .delete('/agents/:id', async (c) => {
      const { id } = c.req.param()

      try {
        await getOpenClawService().removeAgent(id)
        return c.json({ success: true })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        if (message.includes('not found')) {
          return c.json({ error: message }, 404)
        }
        if (message.includes('Cannot delete')) {
          return c.json({ error: message }, 400)
        }
        return c.json({ error: message }, 500)
      }
    })

    .post('/agents/:id/chat', async (c) => {
      const { id } = c.req.param()
      const body = await c.req.json<{
        message: string
        sessionKey?: string
      }>()

      if (!body.message?.trim()) {
        return c.json({ error: 'Message is required' }, 400)
      }

      const sessionKey = body.sessionKey ?? crypto.randomUUID()

      try {
        const eventStream = getOpenClawService().chatStream(
          id,
          sessionKey,
          body.message,
        )

        c.header('Content-Type', 'text/event-stream')
        c.header('Cache-Control', 'no-cache')
        c.header('X-Session-Key', sessionKey)

        return stream(c, async (s) => {
          const reader = eventStream.getReader()
          const encoder = new TextEncoder()
          try {
            while (true) {
              const { done, value } = await reader.read()
              if (done) break
              await s.write(
                encoder.encode(`data: ${JSON.stringify(value)}\n\n`),
              )
            }
            await s.write(encoder.encode('data: [DONE]\n\n'))
          } finally {
            await reader.cancel()
          }
        })
      } catch (err) {
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
        modelId?: string
      }>()

      if (!body.providerType || !body.apiKey) {
        return c.json({ error: 'providerType and apiKey are required' }, 400)
      }

      try {
        await getOpenClawService().updateProviderKeys(
          body.providerType,
          body.apiKey,
        )
        return c.json({
          status: 'restarting',
          message: 'Provider updated, restarting gateway',
        })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ error: message }, 500)
      }
    })
}
