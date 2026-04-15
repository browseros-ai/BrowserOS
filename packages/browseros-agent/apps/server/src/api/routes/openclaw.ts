/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * HTTP routes for OpenClaw agent management.
 * Thin layer delegating to OpenClawService.
 */

import { OPENCLAW_GATEWAY_PORT } from '@browseros/shared/constants/openclaw'
import { BROWSEROS_ROLE_TEMPLATES } from '@browseros/shared/constants/role-aware-agents'
import type {
  BrowserOSAgentRoleId,
  BrowserOSCustomRoleInput,
} from '@browseros/shared/types/role-aware-agents'
import { Hono } from 'hono'
import { stream } from 'hono/streaming'
import { logger } from '../../lib/logger'
import {
  OpenClawAgentAlreadyExistsError,
  OpenClawAgentNotFoundError,
  OpenClawInvalidAgentNameError,
  OpenClawProtectedAgentError,
} from '../services/openclaw/errors'
import { getOpenClawService } from '../services/openclaw/openclaw-service'

function isValidBoundaryMode(
  value: unknown,
): value is BrowserOSCustomRoleInput['boundaries'][number]['defaultMode'] {
  return value === 'allow' || value === 'ask' || value === 'block'
}

function isValidCustomRoleBoundary(value: unknown): boolean {
  if (!value || typeof value !== 'object') return false
  const boundary = value as Record<string, unknown>
  return (
    typeof boundary.key === 'string' &&
    typeof boundary.label === 'string' &&
    typeof boundary.description === 'string' &&
    isValidBoundaryMode(boundary.defaultMode)
  )
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
            port: OPENCLAW_GATEWAY_PORT,
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
        if (message.includes('Podman is not available')) {
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

    .get('/roles', async (c) => {
      return c.json({
        roles: BROWSEROS_ROLE_TEMPLATES.map((role) => ({
          id: role.id,
          name: role.name,
          shortDescription: role.shortDescription,
          longDescription: role.longDescription,
          recommendedApps: role.recommendedApps,
          boundaries: role.boundaries,
          defaultAgentName: role.defaultAgentName,
        })),
      })
    })

    .post('/agents', async (c) => {
      const body = await c.req.json<{
        name: string
        roleId?: BrowserOSAgentRoleId
        customRole?: BrowserOSCustomRoleInput
        providerType?: string
        providerName?: string
        baseUrl?: string
        apiKey?: string
        modelId?: string
      }>()
      const name = body.name?.trim()

      if (!name) {
        return c.json({ error: 'Name is required' }, 400)
      }
      if (body.roleId && body.customRole) {
        return c.json(
          { error: 'Provide either roleId or customRole, not both' },
          400,
        )
      }
      if (
        body.customRole &&
        (!body.customRole.name?.trim() ||
          !body.customRole.shortDescription?.trim() ||
          !body.customRole.longDescription?.trim())
      ) {
        return c.json(
          {
            error:
              'Custom roles require name, shortDescription, and longDescription',
          },
          400,
        )
      }
      if (
        body.customRole &&
        (!Array.isArray(body.customRole.recommendedApps) ||
          !Array.isArray(body.customRole.boundaries))
      ) {
        return c.json(
          {
            error: 'Custom roles require recommendedApps and boundaries arrays',
          },
          400,
        )
      }
      if (
        body.customRole &&
        !body.customRole.recommendedApps.every((app) => typeof app === 'string')
      ) {
        return c.json(
          {
            error: 'Custom role recommendedApps must be an array of strings',
          },
          400,
        )
      }
      if (
        body.customRole &&
        !body.customRole.boundaries.every(isValidCustomRoleBoundary)
      ) {
        return c.json(
          {
            error:
              'Custom role boundaries must include key, label, description, and a valid defaultMode',
          },
          400,
        )
      }

      try {
        const agent = await getOpenClawService().createAgent({
          name,
          roleId: body.roleId,
          customRole: body.customRole,
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
        const eventStream = await getOpenClawService().chatStream(
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
        providerName?: string
        baseUrl?: string
        modelId?: string
      }>()

      if (!body.providerType || !body.apiKey) {
        return c.json({ error: 'providerType and apiKey are required' }, 400)
      }

      try {
        await getOpenClawService().updateProviderKeys(body)
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
