import type { ChatStatus, UIMessage } from 'ai'
import { useCallback, useEffect, useRef } from 'react'
import {
  getResponsePreview,
  normalizeExecutionSteps,
} from '@/lib/execution-history/normalize'
import { upsertConversationExecutionTask } from '@/lib/execution-history/storage'
import { taskChangeKey } from '@/lib/execution-history/task-change-key'
import type {
  ExecutionTaskRecord,
  ExecutionTaskStatus,
} from '@/lib/execution-history/types'
import { sentry } from '@/lib/sentry/sentry'

interface StartExecutionTaskInput {
  conversationId: string
  promptText: string
}

interface FinishExecutionTaskInput {
  responseText?: string
  isAbort?: boolean
  isError?: boolean
}

function createTask(input: StartExecutionTaskInput): ExecutionTaskRecord {
  return {
    id: crypto.randomUUID(),
    conversationId: input.conversationId,
    promptText: input.promptText,
    startedAt: new Date().toISOString(),
    status: 'running',
    actionCount: 0,
    approvalCount: 0,
    deniedCount: 0,
    errorCount: 0,
    steps: [],
  }
}

function getLastUserMessage(messages: UIMessage[]): UIMessage | undefined {
  for (let index = messages.length - 1; index >= 0; index--) {
    if (messages[index]?.role === 'user') {
      return messages[index]
    }
  }
}

function getLastAssistantMessage(messages: UIMessage[]): UIMessage | undefined {
  const lastMessage = messages[messages.length - 1]
  if (lastMessage?.role === 'assistant') {
    return lastMessage
  }
}

function getFinishedStatus(
  input: FinishExecutionTaskInput,
): ExecutionTaskStatus {
  if (input.isError) return 'failed'
  if (input.isAbort) return 'stopped'
  return 'completed'
}

const WRITE_THROTTLE_MS = 400

export function useExecutionHistoryTracker() {
  const activeTaskRef = useRef<ExecutionTaskRecord | null>(null)
  const lastSavedKeyRef = useRef('')
  const pendingWriteRef = useRef<ExecutionTaskRecord | null>(null)
  const writeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const writingRef = useRef(false)

  const drainWrites = useCallback(async () => {
    if (writingRef.current) return
    writingRef.current = true
    try {
      while (pendingWriteRef.current) {
        const task = pendingWriteRef.current
        pendingWriteRef.current = null
        try {
          await upsertConversationExecutionTask(task)
        } catch (error) {
          sentry.captureException(error, {
            extra: {
              message: 'Failed to persist execution history task',
              conversationId: task.conversationId,
              taskId: task.id,
            },
          })
        }
      }
    } finally {
      writingRef.current = false
    }
  }, [])

  const flushWrites = useCallback(() => {
    if (writeTimerRef.current) {
      clearTimeout(writeTimerRef.current)
      writeTimerRef.current = null
    }
    void drainWrites()
  }, [drainWrites])

  // Coalesce storage writes: keep only the newest pending task and write at
  // most one at a time. A streamed turn produces a task update per token, and
  // each write is a full read/modify/write of the whole history store, so
  // during streaming they are batched behind a short timer; terminal updates
  // flush immediately (#1972).
  const scheduleWrite = useCallback(
    (task: ExecutionTaskRecord, immediate: boolean) => {
      pendingWriteRef.current = task
      if (immediate) {
        flushWrites()
        return
      }
      if (writeTimerRef.current) return
      writeTimerRef.current = setTimeout(() => {
        writeTimerRef.current = null
        void drainWrites()
      }, WRITE_THROTTLE_MS)
    },
    [drainWrites, flushWrites],
  )

  const persistTask = useCallback(
    (task: ExecutionTaskRecord, options?: { immediate?: boolean }) => {
      activeTaskRef.current = task
      const immediate = options?.immediate ?? task.status !== 'running'
      const key = taskChangeKey(task)
      if (!immediate && key === lastSavedKeyRef.current) return
      lastSavedKeyRef.current = key
      scheduleWrite(task, immediate)
    },
    [scheduleWrite],
  )

  const startTask = useCallback(
    (input: StartExecutionTaskInput) => {
      const task = createTask(input)
      lastSavedKeyRef.current = ''
      persistTask(task, { immediate: true })
      return task.id
    },
    [persistTask],
  )

  const syncFromMessages = useCallback(
    (messages: UIMessage[], status: ChatStatus) => {
      const activeTask = activeTaskRef.current
      if (!activeTask) return

      const promptMessage = getLastUserMessage(messages)
      const assistantMessage = getLastAssistantMessage(messages)
      const normalized = normalizeExecutionSteps({
        assistantMessage,
        previousSteps: activeTask.steps,
        nowIso: new Date().toISOString(),
      })

      persistTask(
        {
          ...activeTask,
          promptMessageId: activeTask.promptMessageId ?? promptMessage?.id,
          assistantMessageId:
            normalized.assistantMessageId ?? activeTask.assistantMessageId,
          responsePreview:
            getResponsePreview(assistantMessage) || activeTask.responsePreview,
          actionCount: normalized.actionCount,
          approvalCount: normalized.approvalCount,
          deniedCount: normalized.deniedCount,
          errorCount: normalized.errorCount,
          steps: normalized.steps,
        },
        { immediate: status === 'ready' || status === 'error' },
      )
    },
    [persistTask],
  )

  const finishTask = useCallback(
    async (input: FinishExecutionTaskInput) => {
      const activeTask = activeTaskRef.current
      if (!activeTask) return

      const responseText = input.responseText?.trim() || activeTask.responseText
      const nextTask: ExecutionTaskRecord = {
        ...activeTask,
        completedAt: new Date().toISOString(),
        status: getFinishedStatus(input),
        responseText,
        responsePreview: responseText
          ? getResponsePreview({
              parts: [{ type: 'text', text: responseText }],
            } as Pick<UIMessage, 'parts'>)
          : activeTask.responsePreview,
      }

      persistTask(nextTask, { immediate: true })
      activeTaskRef.current = null
    },
    [persistTask],
  )

  const clearActiveTask = useCallback(() => {
    activeTaskRef.current = null
    lastSavedKeyRef.current = ''
    flushWrites()
  }, [flushWrites])

  useEffect(
    () => () => {
      if (writeTimerRef.current) clearTimeout(writeTimerRef.current)
    },
    [],
  )

  return {
    startTask,
    syncFromMessages,
    finishTask,
    clearActiveTask,
  }
}
