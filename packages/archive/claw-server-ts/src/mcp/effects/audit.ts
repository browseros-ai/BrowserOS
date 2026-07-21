/** Persists every executed dispatch, then captures its live session best-effort. */

import { logger } from '../../lib/logger'
import { extractPageId } from '../../lib/tab-activity'
import { recordToolDispatch } from '../../services/audit-log'
import { writeSessionScreenshot } from '../../services/screenshots'
import { sessionVisualService } from '../../services/session-visuals'
import type { ToolEffect } from '../dispatch'

export const applyAudit: ToolEffect = async ({
  call,
  result,
  cancelled,
  durationMs,
}) => {
  if (!call.agent || !call.agentLabel) {
    logger.warn('cockpit dispatch missing identity', {
      tool: call.tool.name,
      sessionId: call.sessionId || undefined,
    })
    return undefined
  }

  const resultPageId = (
    result.structuredContent as { page?: number } | undefined
  )?.page
  const pageId =
    call.flags.newPage && typeof resultPageId === 'number'
      ? resultPageId
      : extractPageId(call.tool.name, call.args)
  const live = pageId !== null ? call.session?.pages.getInfo(pageId) : null
  const page = live ?? call.pageSnapshot
  const dispatchId = recordToolDispatch({
    agentId: call.agent.agentId,
    slug: call.agent.slug,
    agentLabel: call.agentLabel,
    sessionId: call.sessionId,
    toolName: call.tool.name,
    pageId,
    tabId: page?.tabId ?? null,
    targetId: page?.targetId ?? null,
    url: page?.url ?? null,
    title: page?.title ?? null,
    rawArgs: call.args,
    durationMs,
    result: {
      isError: cancelled || (result.isError ?? false),
      structuredContent: result.structuredContent,
      content: result.content,
    },
  })
  if (dispatchId === null) return undefined

  try {
    const bytes = await sessionVisualService.capture(call.sessionId)
    if (bytes) {
      await writeSessionScreenshot(call.sessionId, dispatchId, bytes)
    }
  } catch (error) {
    logger.warn('audit screenshot capture failed', {
      sessionId: call.sessionId,
      dispatchId,
      error: error instanceof Error ? error.message : String(error),
    })
  }
  return undefined
}
