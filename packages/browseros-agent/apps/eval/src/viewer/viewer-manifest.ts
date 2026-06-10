import {
  buildRunMetrics,
  type EvalRunMetrics,
  type EvalTaskMetrics,
} from '../reporting/task-metrics'
import type { GraderResult, TaskDatasetMetadata } from '../types'

const VIEWER_MANIFEST_SCHEMA_VERSION = 3

/** Per-criterion entry surfaced from the agisdk_state_diff grader. */
export interface ViewerCriterion {
  passed: boolean
  softened?: boolean
  /** Free-form string from the grader, OR a structured `{ actual_value, expected_value }` object. */
  detail: unknown
}

export interface ViewerManifestTaskPaths {
  attempt: string
  metadata: string
  messages: string
  trace: string
  grades: string
  screenshots: string
  graderArtifacts: string
  finishState?: string
}

export interface ViewerManifestTaskInput {
  queryId: string
  artifactId?: string
  query: string
  startUrl?: string
  status: string
  durationMs: number
  screenshotCount: number
  metrics?: EvalTaskMetrics
  graderResults: Record<string, GraderResult>
  taskMetadata?: TaskDatasetMetadata
  perCriterion?: ViewerCriterion[]
  finalAnswer?: string | null
}

export interface ViewerManifestTask
  extends Omit<ViewerManifestTaskInput, 'artifactId'> {
  startUrl: string
  paths: ViewerManifestTaskPaths
}

export interface ViewerManifest {
  schemaVersion: typeof VIEWER_MANIFEST_SCHEMA_VERSION
  runId: string
  suiteId?: string
  variantId?: string
  uploadedAt?: string
  reportPath?: string
  agentConfig?: Record<string, unknown>
  dataset?: string
  summary?: Record<string, unknown>
  metrics?: EvalRunMetrics
  tasks: ViewerManifestTask[]
}

export interface BuildViewerManifestInput {
  runId: string
  suiteId?: string
  variantId?: string
  uploadedAt?: string
  reportPath?: string
  agentConfig?: Record<string, unknown>
  dataset?: string
  summary?: Record<string, unknown>
  tasks: ViewerManifestTaskInput[]
}

function taskPaths(queryId: string): ViewerManifestTaskPaths {
  return {
    attempt: `tasks/${queryId}/attempt.json`,
    metadata: `tasks/${queryId}/metadata.json`,
    messages: `tasks/${queryId}/messages.jsonl`,
    trace: `tasks/${queryId}/trace.jsonl`,
    grades: `tasks/${queryId}/grades.json`,
    screenshots: `tasks/${queryId}/screenshots`,
    graderArtifacts: `tasks/${queryId}/grader-artifacts`,
    finishState: `tasks/${queryId}/grader-artifacts/agisdk_state_diff/finish-state.json`,
  }
}

function defaultMetrics(screenshotCount: number): EvalTaskMetrics {
  return {
    durationMs: 0,
    steps: screenshotCount,
    screenshots: screenshotCount,
    toolCalls: 0,
    toolErrors: 0,
    perTool: {},
  }
}

/** Builds the compact JSON index consumed by the static R2 viewer. */
export function buildViewerManifest(
  input: BuildViewerManifestInput,
): ViewerManifest {
  const tasks = input.tasks.map((task) => {
    const { artifactId, ...publicTask } = task
    const metrics =
      publicTask.metrics ??
      ({
        ...defaultMetrics(publicTask.screenshotCount),
        durationMs: publicTask.durationMs,
      } satisfies EvalTaskMetrics)

    return {
      ...publicTask,
      metrics,
      startUrl: publicTask.startUrl ?? '',
      paths: taskPaths(artifactId ?? publicTask.queryId),
    }
  })

  return {
    schemaVersion: VIEWER_MANIFEST_SCHEMA_VERSION,
    runId: input.runId,
    ...(input.suiteId ? { suiteId: input.suiteId } : {}),
    ...(input.variantId ? { variantId: input.variantId } : {}),
    ...(input.uploadedAt ? { uploadedAt: input.uploadedAt } : {}),
    ...(input.reportPath ? { reportPath: input.reportPath } : {}),
    ...(input.agentConfig ? { agentConfig: input.agentConfig } : {}),
    ...(input.dataset ? { dataset: input.dataset } : {}),
    ...(input.summary ? { summary: input.summary } : {}),
    metrics: buildRunMetrics(tasks.map((task) => task.metrics)),
    tasks,
  }
}
