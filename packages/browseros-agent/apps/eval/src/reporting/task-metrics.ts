import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'
import type { TokenUsage } from '../types'

export interface PerToolCounts {
  calls: number
  errors: number
}

export interface PerToolRunStats extends PerToolCounts {
  avgCalls: number
}

export interface CriterionSummary {
  total: number
  passed: number
  softened: number
}

export interface EvalTaskMetrics {
  durationMs: number
  steps: number
  screenshots: number
  toolCalls: number
  toolErrors: number
  perTool: Record<string, PerToolCounts>
  tokenUsage?: TokenUsage
  criteria?: CriterionSummary
  terminationReason?: string
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
  perTool: Record<string, PerToolRunStats>
  tokenUsage?: {
    total: TokenUsage
    avg: TokenUsage
    maxInputOutputTotal?: number
  }
  criteria?: {
    totalCriteria: number
    passedCriteria: number
    softenedCriteria: number
  }
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

/** Strip the MCP transport prefix so different agent backends report the same tool. */
function normalizeToolName(rawName: string): string {
  if (rawName.startsWith('mcp__browseros__')) {
    return rawName.slice('mcp__browseros__'.length)
  }
  if (rawName.startsWith('mcp__')) {
    const parts = rawName.split('__')
    return parts[parts.length - 1] ?? rawName
  }
  return rawName
}

function numberValue(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

interface MessageMetrics {
  toolCalls: number
  toolErrors: number
  perTool: Record<string, PerToolCounts>
}

function countMessageMetrics(messagesJsonl: string): MessageMetrics {
  let toolCalls = 0
  let toolErrors = 0
  const perTool: Record<string, PerToolCounts> = {}
  const toolNameById = new Map<string, string>()

  function bump(name: string, field: keyof PerToolCounts): void {
    const normalized = normalizeToolName(name)
    if (!perTool[normalized]) perTool[normalized] = { calls: 0, errors: 0 }
    perTool[normalized][field]++
  }

  for (const line of messagesJsonl.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) continue
    try {
      const event = JSON.parse(trimmed) as {
        type?: unknown
        toolName?: unknown
        toolCallId?: unknown
      }
      if (event.type === 'tool-input-available') {
        toolCalls++
        const name =
          typeof event.toolName === 'string' ? event.toolName : 'unknown'
        if (typeof event.toolCallId === 'string') {
          toolNameById.set(event.toolCallId, name)
        }
        bump(name, 'calls')
      } else if (event.type === 'tool-output-error') {
        toolErrors++
        const name =
          (typeof event.toolCallId === 'string'
            ? toolNameById.get(event.toolCallId)
            : undefined) ?? 'unknown'
        bump(name, 'errors')
      }
    } catch {
      // Ignore malformed telemetry lines; the raw artifact is still uploaded.
    }
  }

  return { toolCalls, toolErrors, perTool }
}

function extractTokenUsage(
  metadata: Record<string, unknown>,
): TokenUsage | undefined {
  const raw = metadata.token_usage
  if (!raw || typeof raw !== 'object') return undefined
  const obj = raw as Record<string, unknown>
  const usage: TokenUsage = {
    input_tokens: numberValue(obj.input_tokens),
    output_tokens: numberValue(obj.output_tokens),
    cache_read_tokens: numberValue(obj.cache_read_tokens),
    cache_creation_tokens: numberValue(obj.cache_creation_tokens),
  }
  const total =
    usage.input_tokens +
    usage.output_tokens +
    usage.cache_read_tokens +
    usage.cache_creation_tokens
  return total > 0 ? usage : undefined
}

function extractCriterionSummary(
  metadata: Record<string, unknown>,
): CriterionSummary | undefined {
  const graders = metadata.grader_results as
    | Record<string, { details?: { per_criterion?: unknown } }>
    | undefined
  if (!graders) return undefined
  const agisdk = graders.agisdk_state_diff
  const list = agisdk?.details?.per_criterion
  if (!Array.isArray(list) || list.length === 0) return undefined

  let passed = 0
  let softened = 0
  for (const entry of list) {
    if (entry && typeof entry === 'object') {
      const e = entry as { passed?: unknown; softened?: unknown }
      if (e.passed === true) passed++
      if (e.softened === true) softened++
    }
  }
  return { total: list.length, passed, softened }
}

function buildTaskMetrics(
  metadata: Record<string, unknown>,
  messageMetrics: MessageMetrics,
  screenshotCount = 0,
): EvalTaskMetrics {
  const screenshots = numberValue(metadata.screenshot_count) || screenshotCount
  const tokenUsage = extractTokenUsage(metadata)
  const criteria = extractCriterionSummary(metadata)
  const terminationReason =
    typeof metadata.termination_reason === 'string'
      ? metadata.termination_reason
      : undefined
  return {
    durationMs: numberValue(metadata.total_duration_ms),
    steps: numberValue(metadata.total_steps) || screenshots,
    screenshots,
    toolCalls: messageMetrics.toolCalls,
    toolErrors: messageMetrics.toolErrors,
    perTool: messageMetrics.perTool,
    ...(tokenUsage ? { tokenUsage } : {}),
    ...(criteria ? { criteria } : {}),
    ...(terminationReason ? { terminationReason } : {}),
  }
}

function emptyUsage(): TokenUsage {
  return {
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
  }
}

function sumUsage(into: TokenUsage, addition: TokenUsage): void {
  into.input_tokens += addition.input_tokens
  into.output_tokens += addition.output_tokens
  into.cache_read_tokens += addition.cache_read_tokens
  into.cache_creation_tokens += addition.cache_creation_tokens
}

function divUsage(usage: TokenUsage, count: number): TokenUsage {
  if (count <= 0) return emptyUsage()
  return {
    input_tokens: usage.input_tokens / count,
    output_tokens: usage.output_tokens / count,
    cache_read_tokens: usage.cache_read_tokens / count,
    cache_creation_tokens: usage.cache_creation_tokens / count,
  }
}

export function buildRunMetrics(metrics: EvalTaskMetrics[]): EvalRunMetrics {
  const taskCount = metrics.length
  const totalDurationMs = metrics.reduce(
    (sum, metric) => sum + metric.durationMs,
    0,
  )
  const totalSteps = metrics.reduce((sum, metric) => sum + metric.steps, 0)
  const totalToolCalls = metrics.reduce(
    (sum, metric) => sum + metric.toolCalls,
    0,
  )
  const totalToolErrors = metrics.reduce(
    (sum, metric) => sum + metric.toolErrors,
    0,
  )

  // Aggregate per-tool counts across all tasks
  const perToolTotals: Record<string, PerToolCounts> = {}
  for (const m of metrics) {
    for (const [tool, counts] of Object.entries(m.perTool ?? {})) {
      if (!perToolTotals[tool]) perToolTotals[tool] = { calls: 0, errors: 0 }
      perToolTotals[tool].calls += counts.calls
      perToolTotals[tool].errors += counts.errors
    }
  }
  const perTool: Record<string, PerToolRunStats> = {}
  for (const [tool, counts] of Object.entries(perToolTotals)) {
    perTool[tool] = {
      ...counts,
      avgCalls: taskCount > 0 ? counts.calls / taskCount : 0,
    }
  }

  // Aggregate token usage
  let tokenUsage: EvalRunMetrics['tokenUsage']
  const withTokens = metrics.filter(
    (m): m is EvalTaskMetrics & { tokenUsage: TokenUsage } =>
      m.tokenUsage !== undefined,
  )
  if (withTokens.length > 0) {
    const total = emptyUsage()
    let maxInputOutputTotal = 0
    for (const m of withTokens) {
      sumUsage(total, m.tokenUsage)
      const t = m.tokenUsage.input_tokens + m.tokenUsage.output_tokens
      if (t > maxInputOutputTotal) maxInputOutputTotal = t
    }
    tokenUsage = {
      total,
      avg: divUsage(total, withTokens.length),
      maxInputOutputTotal,
    }
  }

  // Aggregate criteria
  let criteria: EvalRunMetrics['criteria']
  const withCriteria = metrics.filter((m) => m.criteria !== undefined)
  if (withCriteria.length > 0) {
    let totalCriteria = 0
    let passedCriteria = 0
    let softenedCriteria = 0
    for (const m of withCriteria) {
      const c = m.criteria
      if (!c) continue
      totalCriteria += c.total
      passedCriteria += c.passed
      softenedCriteria += c.softened
    }
    criteria = { totalCriteria, passedCriteria, softenedCriteria }
  }

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
    perTool,
    ...(tokenUsage ? { tokenUsage } : {}),
    ...(criteria ? { criteria } : {}),
  }
}

export async function readTaskMetrics(
  taskPath: string,
  metadata: Record<string, unknown>,
  screenshotCount = 0,
): Promise<EvalTaskMetrics> {
  const messages = await readFile(join(taskPath, 'messages.jsonl'), 'utf-8')
    .then(countMessageMetrics)
    .catch(
      (): MessageMetrics => ({
        toolCalls: 0,
        toolErrors: 0,
        perTool: {},
      }),
    )
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
