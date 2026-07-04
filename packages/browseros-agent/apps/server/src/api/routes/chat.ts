import type { Browser } from '@browseros/browser-core/browser'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { REMOTE_HERMES_PROVIDER_TYPE } from '@browseros/shared/constants/hermes'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { SessionStore } from '../../agent/session-store'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { Sentry } from '../../lib/sentry'
import { ChatService } from '../services/chat-service'
import type { KlavisService } from '../services/klavis'
import type { RemoteHermesService } from '../services/remote-hermes/remote-hermes-service'
import type { ServerActivity } from '../services/server-activity'
import { ChatRequestSchema } from '../types'
import { resolveBrowserContextPageIds } from '../utils/resolve-browser-context-page-ids'
import { ConversationIdParamSchema } from '../utils/validation'

interface ChatRouteDeps {
  browser: Browser
  browserSession: BrowserSession
  browserosId?: string
  klavis?: KlavisService
  aiSdkDevtoolsEnabled?: boolean
  /** Port the BrowserOS server bound to. Threaded to ACP providers so
   *  the spawned agent can dial back into the local /mcp route. */
  serverPort: number
  /** BrowserOS resources directory. Threaded to ACP providers so the
   *  bundled-Bun launcher under <resourcesDir>/bin/third_party/bun
   *  can be located for built-in adapters (claude / codex). */
  resourcesDir?: string | null
  /** Configured at server startup when AGENT_RUNNER_JWT_SECRET is set.
   *  Null otherwise; `remote-hermes` chat requests get a soft 500. */
  remoteHermes?: RemoteHermesService | null
  activity?: ServerActivity
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
  })

  return new Hono()
    .post('/', zValidator('json', ChatRequestSchema), async (c) => {
      const request = c.req.valid('json')

      // Sentry + metrics (HTTP concerns only)
      Sentry.getCurrentScope().setTag(
        'request-type',
        request.isScheduledTask ? 'schedule' : 'chat',
      )
      Sentry.setContext('request', {
        provider: request.provider,
        model: request.model,
        baseUrl: request.baseUrl
          ? (() => {
              try {
                return new URL(request.baseUrl).origin
              } catch {
                return undefined
              }
            })()
          : undefined,
      })

      metrics.log('chat.request', {
        provider: request.provider,
        model: request.model,
      })

      logger.info('Chat request received', {
        conversationId: request.conversationId,
        provider: request.provider,
        model: request.model,
      })

      if (request.provider === REMOTE_HERMES_PROVIDER_TYPE) {
        if (!deps.remoteHermes) {
          logger.warn(
            'Remote Hermes chat received but service not configured',
            {
              conversationId: request.conversationId,
            },
          )
          return c.json({ error: 'remote_hermes_not_configured' }, 500)
        }
        logger.info('Routing chat to Remote Hermes', {
          conversationId: request.conversationId,
          model: request.model,
        })
        // Resolve Chrome tab IDs to CDP pageIds on the laptop before
        // forwarding — the remote worker has no CDP target visibility
        // and the LLM needs pageIds in the prompt to dispatch browser
        // tools through the WS RPC bridge.
        const remoteBrowserContext = await resolveBrowserContextPageIds(
          deps.browser,
          request.browserContext,
        )
        const response = await deps.remoteHermes.streamTurn(
          {
            conversationId: request.conversationId,
            message: request.message,
            modelId: request.model,
            browserContext: remoteBrowserContext,
            selectedText: request.selectedText,
            selectedTextSource: request.selectedTextSource,
          },
          c.req.raw.signal,
        )
        return (
          deps.activity?.trackChatResponse(response, c.req.raw.signal) ??
          response
        )
      }

      return service.processMessage(request, c.req.raw.signal)
    })
    .delete(
      '/:conversationId',
      zValidator('param', ConversationIdParamSchema),
      async (c) => {
        const { conversationId } = c.req.valid('param')
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
