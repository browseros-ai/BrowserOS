import { storage } from '@wxt-dev/storage'
import { useEffect, useState } from 'react'
import type {
  ConversationExecutionHistory,
  ExecutionHistoryByConversation,
  ExecutionTaskRecord,
} from './types'

export const executionHistoryStorage =
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

export async function getConversationExecutionHistory(
  conversationId: string,
): Promise<ConversationExecutionHistory | null> {
  const current = (await executionHistoryStorage.getValue()) ?? {}
  return current[conversationId] ?? null
}

export async function getExecutionHistoryByConversation(): Promise<ExecutionHistoryByConversation> {
  return (await executionHistoryStorage.getValue()) ?? {}
}

export async function removeConversationExecutionHistory(
  conversationId: string,
): Promise<void> {
  const current = (await executionHistoryStorage.getValue()) ?? {}
  if (!(conversationId in current)) return

  const { [conversationId]: _removed, ...rest } = current
  await executionHistoryStorage.setValue(rest)
}

export async function removeConversationExecutionTask(args: {
  conversationId: string
  taskId: string
}): Promise<void> {
  const current = (await executionHistoryStorage.getValue()) ?? {}
  const history = current[args.conversationId]
  if (!history) return

  const nextTasks = history.tasks.filter((task) => task.id !== args.taskId)
  if (nextTasks.length === history.tasks.length) return

  if (nextTasks.length === 0) {
    const { [args.conversationId]: _removed, ...rest } = current
    await executionHistoryStorage.setValue(rest)
    return
  }

  await executionHistoryStorage.setValue({
    ...current,
    [args.conversationId]: {
      ...history,
      updatedAt: Date.now(),
      tasks: nextTasks,
    },
  })
}

export function useConversationExecutionHistory(conversationId?: string) {
  const [history, setHistory] = useState<ConversationExecutionHistory | null>(
    null,
  )

  useEffect(() => {
    if (!conversationId) {
      setHistory(null)
      return
    }

    getConversationExecutionHistory(conversationId).then(setHistory)
    const unwatch = executionHistoryStorage.watch((nextValue) => {
      setHistory(nextValue?.[conversationId] ?? null)
    })
    return () => unwatch()
  }, [conversationId])

  return history
}

export function useExecutionHistoryByConversation() {
  const [historyByConversation, setHistoryByConversation] =
    useState<ExecutionHistoryByConversation>({})

  useEffect(() => {
    getExecutionHistoryByConversation().then(setHistoryByConversation)
    const unwatch = executionHistoryStorage.watch((nextValue) => {
      setHistoryByConversation(nextValue ?? {})
    })
    return () => unwatch()
  }, [])

  return historyByConversation
}
