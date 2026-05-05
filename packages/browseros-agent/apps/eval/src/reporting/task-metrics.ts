import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'

export interface EvalTaskMetrics {
  durationMs: number
  steps: number
  screenshots: number
  toolCalls: number
  toolErrors: number
}

export interface EvalRunMetrics {
  taskCount: number
  totalDurationMs: number
  avgDurationMs: number
  totalSteps: number
  avgSteps: number
  totalToolCalls: number
  avgToolCalls: number
  totalToolErrors: number
  avgToolErrors: number
}

export interface EvalTaskMetricSummary {
  queryId: string
  status: string
  score?: number
  pass?: boolean
  metrics: EvalTaskMetrics
}

export interface EvalRunMetricSummary {
  run: EvalRunMetrics
  tasks: EvalTaskMetricSummary[]
}

interface TaskDirEntry {
  taskId: string
  taskPath: string
}

function numberValue(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

export function countMessageMetrics(messagesJsonl: string): {
  toolCalls: number
  toolErrors: number
} {
  let toolCalls = 0
  let toolErrors = 0

  for (const line of messagesJsonl.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) continue
    try {
      const event = JSON.parse(trimmed) as { type?: unknown }
      if (event.type === 'tool-input-available') toolCalls++
      if (event.type === 'tool-output-error') toolErrors++
    } catch {
      // Ignore malformed telemetry lines; the raw artifact is still uploaded.
    }
  }

  return { toolCalls, toolErrors }
}

export function buildTaskMetrics(
  metadata: Record<string, unknown>,
  messageMetrics: { toolCalls: number; toolErrors: number },
  screenshotCount = 0,
): EvalTaskMetrics {
  const screenshots = numberValue(metadata.screenshot_count) || screenshotCount
  return {
    durationMs: numberValue(metadata.total_duration_ms),
    steps: numberValue(metadata.total_steps) || screenshots,
    screenshots,
    toolCalls: messageMetrics.toolCalls,
    toolErrors: messageMetrics.toolErrors,
  }
}

export function buildRunMetrics(metrics: EvalTaskMetrics[]): EvalRunMetrics {
  const taskCount = metrics.length
  const totalDurationMs = metrics.reduce((sum, metric) => {
    return sum + metric.durationMs
  }, 0)
  const totalSteps = metrics.reduce((sum, metric) => sum + metric.steps, 0)
  const totalToolCalls = metrics.reduce((sum, metric) => {
    return sum + metric.toolCalls
  }, 0)
  const totalToolErrors = metrics.reduce((sum, metric) => {
    return sum + metric.toolErrors
  }, 0)

  return {
    taskCount,
    totalDurationMs,
    avgDurationMs: taskCount > 0 ? totalDurationMs / taskCount : 0,
    totalSteps,
    avgSteps: taskCount > 0 ? totalSteps / taskCount : 0,
    totalToolCalls,
    avgToolCalls: taskCount > 0 ? totalToolCalls / taskCount : 0,
    totalToolErrors,
    avgToolErrors: taskCount > 0 ? totalToolErrors / taskCount : 0,
  }
}

export async function readTaskMetrics(
  taskPath: string,
  metadata: Record<string, unknown>,
  screenshotCount = 0,
): Promise<EvalTaskMetrics> {
  const messages = await readFile(join(taskPath, 'messages.jsonl'), 'utf-8')
    .then(countMessageMetrics)
    .catch(() => ({ toolCalls: 0, toolErrors: 0 }))
  return buildTaskMetrics(metadata, messages, screenshotCount)
}

function statusFromMetadata(metadata: Record<string, unknown>): string {
  const termination = metadata.termination_reason
  if (termination === 'timeout') return 'timeout'
  if (Array.isArray(metadata.errors) && metadata.errors.length > 0) {
    return 'failed'
  }
  return 'completed'
}

function primaryGrade(metadata: Record<string, unknown>): {
  score?: number
  pass?: boolean
} {
  const graders = metadata.grader_results as
    | Record<string, { score?: unknown; pass?: unknown }>
    | undefined
  const first = graders ? Object.values(graders)[0] : undefined
  return {
    ...(typeof first?.score === 'number' ? { score: first.score } : {}),
    ...(typeof first?.pass === 'boolean' ? { pass: first.pass } : {}),
  }
}

async function readTaskDirs(runDir: string): Promise<TaskDirEntry[]> {
  const canonicalTasksDir = join(runDir, 'tasks')
  const canonicalStat = await stat(canonicalTasksDir).catch(() => null)
  const baseDir = canonicalStat?.isDirectory() ? canonicalTasksDir : runDir
  const entries = await readdir(baseDir, { withFileTypes: true }).catch(
    () => [],
  )

  return entries
    .filter((entry) => entry.isDirectory())
    .filter((entry) => entry.name !== 'screenshots')
    .filter((entry) => entry.name !== 'tasks')
    .map((entry) => ({
      taskId: entry.name,
      taskPath: join(baseDir, entry.name),
    }))
}

export async function readRunMetricSummary(
  runDir: string,
): Promise<EvalRunMetricSummary> {
  const tasks: EvalTaskMetricSummary[] = []

  for (const entry of await readTaskDirs(runDir)) {
    const metadata = await readFile(
      join(entry.taskPath, 'metadata.json'),
      'utf-8',
    )
      .then((text) => JSON.parse(text) as Record<string, unknown>)
      .catch(() => null)
    if (!metadata) continue

    const metrics = await readTaskMetrics(entry.taskPath, metadata)
    tasks.push({
      queryId: (metadata.query_id as string | undefined) || entry.taskId,
      status: statusFromMetadata(metadata),
      ...primaryGrade(metadata),
      metrics,
    })
  }

  return {
    run: buildRunMetrics(tasks.map((task) => task.metrics)),
    tasks,
  }
}
