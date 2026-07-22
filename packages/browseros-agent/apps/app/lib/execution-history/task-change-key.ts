import type { ExecutionTaskRecord } from './types'

/**
 * Cheap change detector for a task: covers step progress, counts, status, and
 * the streamed preview without serializing tool input/output payloads. Used to
 * skip redundant writes instead of `JSON.stringify`-ing the whole task (which,
 * with base64 tool results, was a per-token cost, #1972).
 */
export function taskChangeKey(task: ExecutionTaskRecord): string {
  const lastStep = task.steps[task.steps.length - 1]
  return [
    task.status,
    task.steps.length,
    lastStep?.id ?? '',
    lastStep?.state ?? '',
    lastStep?.previewText ?? '',
    task.actionCount,
    task.approvalCount,
    task.deniedCount,
    task.errorCount,
    task.assistantMessageId ?? '',
    task.completedAt ?? '',
    task.responsePreview ?? '',
  ].join('|')
}
