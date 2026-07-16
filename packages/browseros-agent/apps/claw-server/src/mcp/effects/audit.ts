/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { logger } from '../../lib/logger'
import { extractPageId } from '../../lib/tab-activity'
import { recordToolDispatch } from '../../services/audit-log'
import { persistScreenshot } from '../../services/screenshots'
import type { ToolEffect } from '../dispatch'

/** Persists cancelled or successful dispatches and their screenshot metadata. */
export const applyAudit: ToolEffect = ({
  call,
  result,
  cancelled,
  durationMs,
}) => {
  if (cancelled) {
    if (call.agent && call.agentLabel) {
      recordDispatch(call, result, durationMs, call.agentLabel)
    }
    return undefined
  }
  if (result.isError) return undefined
  if (!call.agent || !call.agentLabel) {
    logger.warn('cockpit dispatch missing identity', {
      tool: call.tool.name,
      sessionId: call.sessionId || undefined,
    })
    return undefined
  }

  const pageId = extractPageId(call.tool.name, call.args)
  const live = pageId !== null ? call.session?.pages.getInfo(pageId) : null
  const dispatchId = recordToolDispatch({
    agentId: call.agent.agentId,
    slug: call.agent.slug,
    agentLabel: call.agentLabel,
    sessionId: call.sessionId,
    toolName: call.tool.name,
    pageId,
    targetId: live?.targetId ?? null,
    url: live?.url ?? null,
    title: live?.title ?? null,
    rawArgs: call.args,
    durationMs,
    result: {
      isError: result.isError ?? false,
      structuredContent: result.structuredContent,
      content: result.content,
    },
  })
  if (dispatchId === null) return undefined

  // `tabs new` is the one page-targeted tool whose page id is born in the
  // result. Prefer it so screenshot fallback and first-capture use that tab.
  const resultPageId = (
    result.structuredContent as { page?: number } | undefined
  )?.page
  const screenshotPageId =
    call.flags.newPage && typeof resultPageId === 'number'
      ? resultPageId
      : pageId
  persistScreenshot({
    dispatchId,
    toolName: call.tool.name,
    pageId: screenshotPageId,
    agentId: call.agent.agentId,
    result: {
      isError: result.isError ?? false,
      content: result.content,
      structuredContent: result.structuredContent,
    },
  })
  return undefined
}

function recordDispatch(
  call: Parameters<ToolEffect>[0]['call'],
  result: Parameters<ToolEffect>[0]['result'],
  durationMs: number,
  agentLabel: string,
): void {
  if (!call.agent) return
  const pageId = extractPageId(call.tool.name, call.args)
  const live = pageId !== null ? call.session?.pages.getInfo(pageId) : null
  recordToolDispatch({
    agentId: call.agent.agentId,
    slug: call.agent.slug,
    agentLabel,
    sessionId: call.sessionId,
    toolName: call.tool.name,
    pageId,
    targetId: live?.targetId ?? null,
    url: live?.url ?? null,
    title: live?.title ?? null,
    rawArgs: call.args,
    durationMs,
    result: {
      isError: true,
      structuredContent: result.structuredContent,
      content: result.content,
    },
  })
}
