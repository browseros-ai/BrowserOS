/** Persists every executed dispatch, then captures its live session best-effort. */

import { logger } from '../../../lib/logger'
import { extractPageId } from '../../../lib/tab-activity'
import {
  type RecordToolDispatchInput,
  recordToolDispatch,
} from '../../../services/audit-log'
import { writeSessionScreenshot } from '../../../services/screenshots'
import { sessionVisualService } from '../../../services/session-visuals'
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
  await persistAuditDispatch({
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
  return undefined
}

type LocalToolDispatchInput = Omit<
  RecordToolDispatchInput,
  'pageId' | 'tabId' | 'targetId' | 'url' | 'title'
>

/** Records a Claw-local tool without capturing an unrelated browser page. */
export function recordLocalToolDispatch(input: LocalToolDispatchInput): void {
  recordToolDispatch({
    ...input,
    pageId: null,
    tabId: null,
    targetId: null,
    url: null,
    title: null,
  })
}

async function persistAuditDispatch(
  input: RecordToolDispatchInput,
): Promise<void> {
  const dispatchId = recordToolDispatch(input)
  if (dispatchId === null) return undefined

  try {
    const bytes = await sessionVisualService.capture(input.sessionId)
    if (bytes) {
      await writeSessionScreenshot(input.sessionId, dispatchId, bytes)
    }
  } catch (error) {
    logger.warn('audit screenshot capture failed', {
      sessionId: input.sessionId,
      dispatchId,
      error: error instanceof Error ? error.message : String(error),
    })
  }
}
