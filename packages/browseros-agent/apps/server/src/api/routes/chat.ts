import type { Browser } from '@browseros/browser-core/browser'
import type { ConversationPanelAssignments } from '@browseros/shared/schemas/conversation-panels'
import { zValidator } from '@hono/zod-validator'
import { createUIMessageStreamResponse } from 'ai'
import { Hono } from 'hono'
import { SessionStore } from '../../agent/session-store'
import type { AcpAgentRuntime } from '../../lib/agents/acp/acp-agent-runtime'
import { SERVER_CREDENTIALED_PROVIDERS } from '../../lib/clients/llm/config'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { dbProviderStore } from '../../lib/providers/provider-store'
import { Sentry } from '../../lib/sentry'
import {
  type ChatProviderLookup,
  hydrateChatProvider,
} from '../services/chat-provider-config'
import { ChatService } from '../services/chat-service'
import type { ConversationRuns } from '../services/conversation-runs'
import type { KlavisService } from '../services/klavis'
import type { BrowserMcpModule } from '../services/mcp/browser-mcp-module'
import type { ServerActivity } from '../services/server-activity'
import {
  type AcpChatRequest,
  type BrowserOsChatRequest,
  type ChatRequest,
  ChatRequestSchema,
  type Env,
  type HydratedChatRequest,
} from '../types'
import { isTrustedAppRequest } from '../utils/request-auth'
import { ConversationIdParamSchema } from '../utils/validation'

interface ChatRouteDeps {
  browser: Browser
  browserMcp: BrowserMcpModule
  browserosId?: string
  klavis?: KlavisService
  aiSdkDevtoolsEnabled?: boolean
  serverPort: number
  resourcesDir?: string | null
  activity?: ServerActivity
  acpRuntime?: AcpAgentRuntime
  conversationRuns?: ConversationRuns
  /** Injectable so the hydration path is testable without a database. */
  providerStore?: ChatProviderLookup
}

// /chat deliberately exposes a plain Hono type. Its AI SDK stream payloads are
// not an RPC contract, and carrying every inferred route through the root app
// exceeds TypeScript's instantiation depth.
export function createChatRoutes(deps: ChatRouteDeps): Hono<Env> {
  const { browserosId } = deps

  const sessionStore = new SessionStore()
  const service = new ChatService({
    sessionStore,
    klavis: deps.klavis,
    browser: deps.browser,
    browserMcp: deps.browserMcp,
    browserosId,
    aiSdkDevtoolsEnabled: deps.aiSdkDevtoolsEnabled,
    serverPort: deps.serverPort,
    resourcesDir: deps.resourcesDir,
    activity: deps.activity,
    acpRuntime: deps.acpRuntime,
    conversationRuns: deps.conversationRuns,
  })

  const app = new Hono<Env>()
  app.post('/', zValidator('json', ChatRequestSchema), async (c) => {
    const parsed = c.req.valid('json')
    const parsedBrowserRequest = isBrowserOsChatRequest(parsed) ? parsed : null
    if (!parsedBrowserRequest && !isTrustedAppRequest(c)) {
      return c.json({ error: 'Forbidden' }, 403)
    }

    // The provider configuration is filled from the stored row here rather than
    // shipped on every message. A client from before this change sends it all
    // inline and simply finds nothing to overlay.
    let request: HydratedChatRequest
    if (parsedBrowserRequest) {
      const hydrated = await hydrateChatProvider(
        parsedBrowserRequest,
        deps.providerStore ?? dbProviderStore,
      )
      if (!hydrated.ok) return c.json({ error: hydrated.error }, 400)
      // A browseros request is otherwise allowed without the app-origin check,
      // on the reasoning that it carries its own credentials and so can only
      // spend what the caller already held.
      //
      // Two things break that reasoning, and both have to be caught. Naming a
      // stored provider has the server supply the key. So does naming one of
      // the provider types the server credentials itself: the oauth three take
      // a token from this machine's store and browseros takes the gateway
      // credential, none of which the request carries. A caller that genuinely
      // brought its own key is as unrestricted as it was before.
      const usesServerCredentials =
        hydrated.usedStoredProvider ||
        SERVER_CREDENTIALED_PROVIDERS.has(hydrated.request.provider)
      if (usesServerCredentials && !isTrustedAppRequest(c)) {
        return c.json({ error: 'Forbidden' }, 403)
      }
      request = hydrated.request
    } else {
      request = parsed as AcpChatRequest
    }
    const browserRequest = isBrowserOsChatRequest(request) ? request : null

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
  app.get('/panels', (c) => {
    if (!isTrustedAppRequest(c)) {
      return c.json({ error: 'Forbidden' }, 403)
    }
    return panelAssignmentsResponse(service.subscribePanelAssignments())
  })
  app.get('/:conversationId/state', async (c) => {
    // Keep this handler's type shallow: combining an awaited response with the
    // Zod middleware overload exceeds TypeScript's depth in the composed API.
    const parsed = ConversationIdParamSchema.safeParse(c.req.param())
    if (!parsed.success)
      return c.json({ error: 'Invalid conversation id' }, 400)
    const snapshot = await service.getRunSnapshot(parsed.data.conversationId)
    if (!snapshot) return c.json({ error: 'Conversation not found' }, 404)
    c.header('Cache-Control', 'no-store')
    return c.json(snapshot)
  })
  app.get(
    '/:conversationId/stream',
    zValidator('param', ConversationIdParamSchema),
    (c) => {
      const { conversationId } = c.req.valid('param')
      // The run may finish after a panel hydrates `state` but before this GET.
      // Completed records still replay their buffered chunks, closing that race
      // without making the panel reconstruct an assistant message itself.
      const stream = service.subscribe(conversationId)
      return stream
        ? createUIMessageStreamResponse({ stream })
        : new Response(null, { status: 204 })
    },
  )
  app.post(
    '/:conversationId/stop',
    zValidator('param', ConversationIdParamSchema),
    async (c) => {
      const { conversationId } = c.req.valid('param')
      return c.json({ stopped: await service.stop(conversationId) })
    },
  )
  app.delete(
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
  return app
}

function isBrowserOsChatRequest(
  request: ChatRequest,
): request is BrowserOsChatRequest {
  return request.target.type === 'browseros'
}

/** Encodes the current background-only panel assignments as reconnectable SSE. */
function panelAssignmentsResponse(
  stream: ReadableStream<ConversationPanelAssignments>,
): Response {
  const encoder = new TextEncoder()
  const body = stream.pipeThrough(
    new TransformStream<ConversationPanelAssignments, Uint8Array>({
      transform(assignments, controller) {
        controller.enqueue(
          encoder.encode(`data: ${JSON.stringify(assignments)}\n\n`),
        )
      },
    }),
  )
  return new Response(body, {
    headers: {
      'Cache-Control': 'no-cache, no-transform',
      Connection: 'keep-alive',
      'Content-Type': 'text/event-stream',
    },
  })
}
