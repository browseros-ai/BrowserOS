import { type TaskDetail, useTaskDetail } from '@/modules/api/audit.hooks'

export interface TaskDetailScreenData {
  task: TaskDetail | undefined
  isPending: boolean
  isError: boolean
  error: Error | null
}

export function useTaskDetailScreenData(
  sessionId: string,
): TaskDetailScreenData {
  const query = useTaskDetail({ variables: { sessionId } })
  return {
    task: query.data,
    isPending: query.isPending,
    isError: query.isError,
    error: (query.error as Error | null) ?? null,
  }
}
