import { storage } from '@wxt-dev/storage'
import type { ExecutionTaskRecord } from '@/lib/execution-history/types'
import { normalizeToolSequence } from './advisor'
import type {
  WorkflowUsageRecord,
  WorkflowUsageSource,
  WorkflowUsageStore,
} from './types'

const MAX_WORKFLOW_USAGE_RECORDS = 300

export const workflowUsageStorage = storage.defineItem<WorkflowUsageStore>(
  'local:workflowUsagePatterns',
  {
    fallback: { version: 1, records: [] },
    version: 1,
  },
)

export function createWorkflowUsageRecord(input: {
  id: string
  source: WorkflowUsageSource
  toolNames: string[]
  recordedAt?: number
}): WorkflowUsageRecord | null {
  const toolNames = normalizeToolSequence(input.toolNames)
  if (toolNames.length === 0) return null

  return {
    id: input.id,
    source: input.source,
    recordedAt: input.recordedAt ?? Date.now(),
    toolNames,
  }
}

export function createWorkflowUsageRecordFromExecutionTask(
  task: ExecutionTaskRecord,
): WorkflowUsageRecord | null {
  return createWorkflowUsageRecord({
    id: `execution-task:${task.id}`,
    source: 'sidepanel-chat',
    recordedAt: Date.parse(task.completedAt ?? task.startedAt),
    toolNames: task.steps.map((step) => step.toolName),
  })
}

export async function recordWorkflowUsage(
  record: WorkflowUsageRecord | null,
): Promise<void> {
  if (!record) return

  const current = (await workflowUsageStorage.getValue()) ?? {
    version: 1,
    records: [],
  }
  const recordsById = new Map(
    current.records.map((existing) => [existing.id, existing]),
  )
  recordsById.set(record.id, record)

  const records = Array.from(recordsById.values())
    .sort((left, right) => left.recordedAt - right.recordedAt)
    .slice(-MAX_WORKFLOW_USAGE_RECORDS)

  await workflowUsageStorage.setValue({ version: 1, records })
}

export async function getWorkflowUsageRecords(): Promise<
  WorkflowUsageRecord[]
> {
  const current = await workflowUsageStorage.getValue()
  return current?.records ?? []
}

export async function clearWorkflowUsageRecords(): Promise<void> {
  await workflowUsageStorage.setValue({ version: 1, records: [] })
}
