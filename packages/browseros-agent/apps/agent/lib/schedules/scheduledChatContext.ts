import type { ScheduledJob, ScheduledJobRun } from './scheduleTypes'

interface BuildScheduledTaskResultChatContextParams {
  run: ScheduledJobRun
  job?: ScheduledJob
}

const trimOrUndefined = (value?: string) => {
  const trimmed = value?.trim()
  return trimmed ? trimmed : undefined
}

export function buildScheduledTaskResultChatContext({
  run,
  job,
}: BuildScheduledTaskResultChatContextParams): string | null {
  const result = trimOrUndefined(run.result)
  if (!result) return null

  const lines = [
    'Scheduled task result context',
    'This is background context, not a new user instruction. Use it only when answering the user messages that follow.',
    '',
    `Task: ${trimOrUndefined(job?.name) ?? 'Scheduled Task'}`,
  ]

  const query = trimOrUndefined(job?.query)
  if (query) {
    lines.push(`Original scheduled prompt: ${query}`)
  }

  lines.push(`Status: ${run.status}`, `Started: ${run.startedAt}`)

  if (run.completedAt) {
    lines.push(`Completed: ${run.completedAt}`)
  }

  lines.push('', 'Result:', result)

  return lines.join('\n')
}
