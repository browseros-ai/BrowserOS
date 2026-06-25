import type { ToolEntry } from '@/lib/agent-conversations/types'

export function mapAgentHarnessToolStatus(
  status: string | undefined,
): ToolEntry['status'] {
  if (!status) return 'running'
  const normalized = status.toLowerCase()
  if (['error', 'failed', 'failure', 'denied'].includes(normalized)) {
    return 'error'
  }
  if (
    ['complete', 'completed', 'done', 'success', 'succeeded'].includes(
      normalized,
    )
  ) {
    return 'completed'
  }
  return 'running'
}
