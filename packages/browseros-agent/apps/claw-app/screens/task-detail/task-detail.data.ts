import type { SessionScreenshot } from '@browseros/claw-api'
import { type DescribedStep, describeSteps } from '@/lib/step-sentences'
import { useAgentSettings } from '@/modules/api/agent-settings.hooks'
import {
  type TaskDetail,
  useSessionDetail,
  useSessionScreenshots,
} from '@/modules/api/audit.hooks'
import {
  type ReplaySession,
  useReplaySession,
} from '@/screens/replay/use-replay-session'
import { pagesVisited, type VisitedPage } from './task-detail.helpers'

export interface TaskDetailScreenData {
  detail: TaskDetail | undefined
  screenshots: SessionScreenshot[]
  /** The steps a person reads, with behind-the-scenes work left out. */
  steps: DescribedStep[]
  /** Every step, behind-the-scenes work included and flagged. */
  allSteps: DescribedStep[]
  hiddenCount: number
  pages: VisitedPage[]
  replay: ReplaySession
  /** False only once settings say the agents may not ask for help. */
  humanHelpEnabled: boolean
  isPending: boolean
  isError: boolean
  error: Error | null
}

export function useTaskDetailScreenData(
  sessionId: string,
): TaskDetailScreenData {
  const detailQuery = useSessionDetail({ variables: { sessionId } })
  // Poll screenshots only while the session is live; a finished or cancelled
  // session keeps its final snapshot, so stop firing requests.
  const isLive = detailQuery.data?.session.status === 'live'
  const screenshotsQuery = useSessionScreenshots({
    variables: { sessionId },
    refetchInterval: isLive ? 3000 : false,
  })
  const replay = useReplaySession(sessionId)
  const settings = useAgentSettings()

  const dispatches = detailQuery.data?.dispatches ?? []
  const { steps, hiddenCount } = describeSteps(dispatches)
  const { steps: allSteps } = describeSteps(dispatches, { includeHidden: true })

  return {
    detail: detailQuery.data,
    screenshots: screenshotsQuery.data?.items ?? [],
    steps,
    allSteps,
    hiddenCount,
    pages: pagesVisited(dispatches),
    replay,
    humanHelpEnabled: settings.data?.humanHelpEnabled ?? true,
    isPending: detailQuery.isPending || screenshotsQuery.isPending,
    isError: detailQuery.isError || screenshotsQuery.isError,
    error:
      (detailQuery.error as Error | null) ??
      (screenshotsQuery.error as Error | null) ??
      null,
  }
}
