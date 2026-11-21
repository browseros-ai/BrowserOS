import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import {
  TOOL_NOT_RUN,
  TOOL_STOPPED,
} from '@/lib/messaging/server/tool-output-status'
import type { ToolInvocationInfo } from './getMessageSegments'
import { ToolBatch } from './ToolBatch'

const completed: ToolInvocationInfo = {
  toolCallId: 'completed',
  toolName: 'completed_action',
  state: 'output-available',
  input: {},
  output: 'Success',
}
const failed: ToolInvocationInfo = {
  ...completed,
  toolCallId: 'failed',
  toolName: 'failed_action',
  state: 'output-error',
  output: undefined,
}
const stopped: ToolInvocationInfo = {
  ...failed,
  toolCallId: 'stopped',
  toolName: 'stopped_action',
  skippedStatus: TOOL_STOPPED,
}
const notRun: ToolInvocationInfo = {
  ...failed,
  toolCallId: 'not-run',
  toolName: 'queued_action',
  skippedStatus: TOOL_NOT_RUN,
}

const renderBatch = (...tools: ToolInvocationInfo[]) =>
  renderToStaticMarkup(
    <ToolBatch tools={tools} isLastBatch isLastMessage isStreaming />,
  )

describe('ToolBatch', () => {
  test.each([
    [stopped, TOOL_STOPPED],
    [notRun, TOOL_NOT_RUN],
  ])(
    'shows a neutral, accessible status for a skipped action',
    (tool, label) => {
      const html = renderBatch(tool)

      expect(html).toContain(`1 action ${label.toLowerCase()}`)
      expect(html).toContain(`aria-label="${label}"`)
      expect(html).toContain(`>${label}</span>`)
      expect(html).toContain('lucide-circle-dashed')
      expect(html).not.toContain('text-destructive')
    },
  )

  test('shows a failed action with the existing failure icon and summary', () => {
    const html = renderBatch(failed)
    expect(html).toContain('0/1 actions completed')
    expect(html).toContain('lucide-circle-x')
    expect(html).toContain('text-destructive')
  })

  test('shows a completed action with the existing success icon and summary', () => {
    const html = renderBatch(completed)
    expect(html).toContain('1/1 actions completed')
    expect(html).toContain('lucide-circle-check')
    expect(html).toContain('text-green-500')
  })

  test('excludes skipped actions from the completion ratio in a mixed batch', () => {
    const html = renderBatch(completed, failed, stopped, notRun)
    expect(html).toContain('1/2 actions completed · 1 stopped · 1 not run')
  })
})
