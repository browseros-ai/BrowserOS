import type { SessionScreenshot } from '@browseros/claw-api'
import {
  type TaskDetail,
  useSessionDetail,
  useSessionScreenshots,
} from '@/modules/api/audit.hooks'

export interface TaskDetailScreenData {
  detail: TaskDetail | undefined
  screenshots: SessionScreenshot[]
  isPending: boolean
  isError: boolean
  error: Error | null
}

export function useTaskDetailScreenData(
  sessionId: string,
): TaskDetailScreenData {
  const detailQuery = useSessionDetail({ variables: { sessionId } })
  const screenshotsQuery = useSessionScreenshots({ variables: { sessionId } })
  return {
    detail: detailQuery.data,
    screenshots: screenshotsQuery.data?.items ?? [],
    isPending: detailQuery.isPending || screenshotsQuery.isPending,
    isError: detailQuery.isError || screenshotsQuery.isError,
    error:
      (detailQuery.error as Error | null) ??
      (screenshotsQuery.error as Error | null) ??
      null,
  }
}
