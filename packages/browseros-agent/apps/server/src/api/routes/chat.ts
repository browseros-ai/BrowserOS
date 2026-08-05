import type { Browser } from '@browseros/browser-core/browser'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { SessionStore } from '../../agent/session-store'
import type { AcpAgentRuntime } from '../../lib/agents/acp/acp-agent-runtime'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { Sentry } from '../../lib/sentry'
import { ChatService } from '../services/chat-service'
import type { KlavisService } from '../services/klavis'
import type { ServerActivity } from '../services/server-activity'
import {
  type BrowserOsChatRequest,
  type ChatRequest,
  ChatRequestSchema,
  type Env,
} from '../types'
import { isTrustedAppRequest } from '../utils/request-auth'
import { ConversationIdParamSchema } from '../utils/validation'

interface ChatRouteDeps {
  browser: Browser
  browserSession: BrowserSession
  browserosId?: string
  klavis?: KlavisService
  aiSdkDevtoolsEnabled?: boolean
  serverPort: number
  resourcesDir?: string | null
  activity?: ServerActivity
  acpRuntime?: AcpAgentRuntime
}

export function createChatRoutes(deps: ChatRouteDeps) {
  const { browserosId } = deps

  const sessionStore = new SessionStore()
  const service = new ChatService({
    sessionStore,
    klavis: deps.klavis,
    browser: deps.browser,
    browserSession: deps.browserSession,
    browserosId,
    aiSdkDevtoolsEnabled: deps.aiSdkDevtoolsEnabled,
    serverPort: deps.serverPort,
    resourcesDir: deps.resourcesDir,
    activity: deps.activity,
    acpRuntime: deps.acpRuntime,
  })

  return new Hono<Env>()
    .post('/', zValidator('json', ChatRequestSchema), async (c) => {
      const request = c.req.valid('json')
      const browserRequest = isBrowserOsChatRequest(request) ? request : null
      if (!browserRequest && !isTrustedAppRequest(c)) {
        return c.json({ error: 'Forbidden' }, 403)
      }
      const provider = browserRequest?.provider ?? request.target.type
      const model = browserRequest?.model
      const baseUrl = browserRequest?.baseUrl

      Sentry.getCurrentScope().setTag(
        'request-type',
        request.isScheduledTask ? 'schedule' : 'chat',
      )
      Sentry.setContext('request', {
        provider,
        model,
        baseUrl: baseUrl
          ? (() => {
              try {
                return new URL(baseUrl).origin
              } catch {
                return undefined
              }
            })()
          : undefined,
      })

      metrics.log('chat.request', {
        provider,
        model,
      })

      logger.info('Chat request received', {
        conversationId: request.conversationId,
        provider,
        model,
      })

      return service.processMessage(request, c.req.raw.signal)
    })
    .delete(
      '/:conversationId',
      zValidator('param', ConversationIdParamSchema),
      async (c) => {
        const { conversationId } = c.req.valid('param')
        if (service.isAcpSession(conversationId) && !isTrustedAppRequest(c)) {
          return c.json({ error: 'Forbidden' }, 403)
        }
        const result = await service.deleteSession(conversationId)

        if (result.deleted) {
          return c.json({
            success: true,
            message: `Session ${conversationId} deleted`,
            sessionCount: result.sessionCount,
          })
        }

        return c.json(
          { success: false, message: `Session ${conversationId} not found` },
          404,
        )
      },
    )
}

function isBrowserOsChatRequest(
  request: ChatRequest,
): request is BrowserOsChatRequest {
  return request.target.type === 'browseros'
}
