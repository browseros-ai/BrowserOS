import {
  buildRunMetrics,
  type EvalRunMetrics,
  type EvalTaskMetrics,
} from '../reporting/task-metrics'
import type { GraderResult } from '../types'

export const VIEWER_MANIFEST_SCHEMA_VERSION = 2

export interface ViewerManifestTaskPaths {
  attempt: string
  metadata: string
  messages: string
  trace: string
  grades: string
  screenshots: string
  graderArtifacts: string
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
        durationMs: publicTask.durationMs,
        steps: publicTask.screenshotCount,
        screenshots: publicTask.screenshotCount,
        toolCalls: 0,
        toolErrors: 0,
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
