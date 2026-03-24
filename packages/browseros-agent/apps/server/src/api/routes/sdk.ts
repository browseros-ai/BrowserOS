/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * SDK Routes - REST API for @browseros-ai/agent-sdk
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { stream } from 'hono/streaming'
import { logger } from '../../lib/logger'
import { BrowserService } from '../services/sdk/browser'
import { ChatService } from '../services/sdk/chat'
import { ExtractService } from '../services/sdk/extract'
import {
  ActRequestSchema,
  ExtractRequestSchema,
  NavRequestSchema,
  type SdkDeps,
  SdkError,
  VerifyRequestSchema,
} from '../services/sdk/types'
import { VerifyService } from '../services/sdk/verify'
import type { Env } from '../types'
import {
  formatUIMessageStreamDone,
  formatUIMessageStreamEvent,
} from '../utils/ui-message-stream'

async function waitForPageLoad(
  browserService: BrowserService,
  tabId: number,
): Promise<void> {
  const startTime = Date.now()

  while (Date.now() - startTime < TIMEOUTS.PAGE_LOAD_WAIT) {
    const status = await browserService.getPageLoadStatus(tabId)
    if (status.isPageComplete && status.isDOMContentLoaded) {
      logger.debug('Page load complete', { tabId })
      return
    }
    await new Promise((resolve) =>
      setTimeout(resolve, TIMEOUTS.PAGE_LOAD_POLL_INTERVAL),
    )
  }
}

export function createSdkRoutes(deps: SdkDeps) {
  const { port, browser, browserosId } = deps

  const browserService = new BrowserService(browser)
  const chatService = new ChatService(port)
  const extractService = new ExtractService()
  const verifyService = new VerifyService()

  // Chain route definitions for proper Hono RPC type inference
  return new Hono<Env>()
    .post('/nav', zValidator('json', NavRequestSchema), async (c) => {
      const { url, tabId, windowId } = c.req.valid('json')
      logger.info('SDK nav request', { url, tabId, windowId })

      try {
        const { tabId: navigatedTabId } = await browserService.navigate(
          url,
          tabId,
          windowId,
        )

        await waitForPageLoad(browserService, navigatedTabId)

        return c.json({ success: true, tabId: navigatedTabId })
      } catch (error) {
        const err =
          error instanceof SdkError
            ? error
            : new SdkError(
                error instanceof Error ? error.message : 'Navigation failed',
              )
        logger.error('SDK nav error', { url, error: err.message })
        return c.json(
          { error: { message: err.message } },
          err.statusCode as 400 | 500,
        )
      }
    })
    .post('/act', zValidator('json', ActRequestSchema), async (c) => {
      const { instruction, context, browserContext, llm, sessionId, source } =
        c.req.valid('json')
      logger.info('SDK act request', {
        instruction,
        windowId: browserContext?.windowId,
        hasSessionId: !!sessionId,
        source,
      })

      const llmConfig = llm ?? { provider: LLM_PROVIDERS.BROWSEROS }

      if (llmConfig.provider !== LLM_PROVIDERS.BROWSEROS && !llmConfig.model) {
        return c.json(
          {
            error: { message: 'model is required for non-browseros providers' },
          },
          400,
        )
      }

      // Set SSE headers for Vercel AI stream
      c.header('Content-Type', 'text/event-stream')
      c.header('Cache-Control', 'no-cache')
      c.header('Connection', 'keep-alive')
      c.header('x-vercel-ai-ui-message-stream', 'v1')

      return stream(c, async (honoStream) => {
        try {
          // Emit start event at route level
          await honoStream.write(formatUIMessageStreamEvent({ type: 'start' }))

          await chatService.executeAction({
            instruction,
            context,
            browserContext,
            source,
            llmConfig,
            signal: c.req.raw.signal,
            sessionId,
            onSSEEvent: async (event) => {
              // Events from AI agent are already properly formatted
              // Skip start/finish (managed at route level), forward everything else
              if (event.type === 'start' || event.type === 'finish') {
                return
              }
              await honoStream.write(formatUIMessageStreamEvent(event))
            },
          })

          // Emit finish at route level
          await honoStream.write(
            formatUIMessageStreamEvent({
              type: 'finish',
              finishReason: 'stop',
            }),
          )
        } catch (error) {
          if (error instanceof Error && error.name === 'AbortError') {
            await honoStream.write(
              formatUIMessageStreamEvent({ type: 'abort' }),
            )
            await honoStream.write(formatUIMessageStreamDone())
            return
          }
          const err =
            error instanceof SdkError
              ? error
              : new SdkError(
                  error instanceof Error
                    ? error.message
                    : 'Action execution failed',
                )
          logger.error('SDK act error', { instruction, error: err.message })
          await honoStream.write(
            formatUIMessageStreamEvent({
              type: 'error',
              errorText: err.message,
            }),
          )
          await honoStream.write(
            formatUIMessageStreamEvent({
              type: 'finish',
              finishReason: 'error',
            }),
          )
        } finally {
          await honoStream.write(formatUIMessageStreamDone())
        }
      })
    })
    .post('/extract', zValidator('json', ExtractRequestSchema), async (c) => {
      const {
        instruction,
        schema,
        context,
        windowId,
        tabId: requestTabId,
      } = c.req.valid('json')
      logger.info('SDK extract request', {
        instruction,
        windowId,
        tabId: requestTabId,
      })

      try {
        // Use provided tabId, or get active tab (from window if specified)
        const tabId =
          requestTabId ?? (await browserService.getActiveTab(windowId)).tabId
        const content = await browserService.getPageContent(tabId)
        const data = await extractService.extract({
          instruction,
          schema,
          content,
          context,
        })
        return c.json({ data })
      } catch (error) {
        const err =
          error instanceof SdkError
            ? error
            : new SdkError(
                error instanceof Error ? error.message : 'Extraction failed',
              )
        logger.error('SDK extract error', { instruction, error: err.message })
        return c.json(
          { error: { message: err.message } },
          err.statusCode as 400 | 500,
        )
      }
    })
    .post('/verify', zValidator('json', VerifyRequestSchema), async (c) => {
      const {
        expectation,
        context,
        windowId,
        tabId: requestTabId,
        llm,
      } = c.req.valid('json')
      logger.info('SDK verify request', {
        expectation,
        windowId,
        tabId: requestTabId,
      })

      const llmConfig = llm ?? { provider: LLM_PROVIDERS.BROWSEROS }

      try {
        // Use provided tabId, or get active tab (from window if specified)
        const tabId =
          requestTabId ?? (await browserService.getActiveTab(windowId)).tabId
        const [screenshot, interactiveElements] = await Promise.all([
          browserService.getScreenshot(tabId),
          browserService.getInteractiveElements(tabId, true),
        ])

        const result = await verifyService.verify({
          expectation,
          screenshot,
          interactiveElements: interactiveElements.content,
          context,
          llmConfig,
          browserosId,
        })

        return c.json(result)
      } catch (error) {
        const err =
          error instanceof SdkError
            ? error
            : new SdkError(
                error instanceof Error ? error.message : 'Verification failed',
              )
        logger.error('SDK verify error', { expectation, error: err.message })
        return c.json(
          { error: { message: err.message } },
          err.statusCode as 400 | 500,
        )
      }
    })
}
