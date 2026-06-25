import { storage } from '@wxt-dev/storage'
import type {
  ConversationExecutionHistory,
  ExecutionHistoryByConversation,
  ExecutionTaskRecord,
} from './types'

const executionHistoryStorage =
  storage.defineItem<ExecutionHistoryByConversation>(
    'local:executionHistoryByConversation',
    {
      fallback: {},
      version: 1,
    },
  )

function upsertTaskInHistory(
  history: ConversationExecutionHistory,
  task: ExecutionTaskRecord,
): ConversationExecutionHistory {
  const existingIndex = history.tasks.findIndex((item) => item.id === task.id)
  if (existingIndex === -1) {
    return {
      ...history,
      updatedAt: Date.now(),
      tasks: [...history.tasks, task],
    }
  }

  const nextTasks = [...history.tasks]
  nextTasks[existingIndex] = task
  return {
    ...history,
    updatedAt: Date.now(),
    tasks: nextTasks,
  }
}

function createConversationHistory(
  conversationId: string,
): ConversationExecutionHistory {
  return {
    conversationId,
    updatedAt: Date.now(),
    tasks: [],
  }
}

export async function upsertConversationExecutionTask(
  task: ExecutionTaskRecord,
): Promise<void> {
  const current = (await executionHistoryStorage.getValue()) ?? {}
  const history =
    current[task.conversationId] ??
    createConversationHistory(task.conversationId)

  await executionHistoryStorage.setValue({
    ...current,
    [task.conversationId]: upsertTaskInHistory(history, task),
  })
}

export async function removeConversationExecutionHistory(
  conversationId: string,
): Promise<void> {
  const current = (await executionHistoryStorage.getValue()) ?? {}
  if (!(conversationId in current)) return

  const { [conversationId]: _removed, ...rest } = current
  await executionHistoryStorage.setValue(rest)
}
