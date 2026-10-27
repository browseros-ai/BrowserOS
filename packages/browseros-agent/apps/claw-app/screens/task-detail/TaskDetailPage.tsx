import { useQueryClient } from '@tanstack/react-query'
import { Lightbulb } from 'lucide-react'
import { type ComponentProps, useRef, useState } from 'react'
import { useParams } from 'react-router'
import { StepLogDialog } from '@/components/audit/StepLogDialog'
import { EmptyState } from '@/components/cockpit/EmptyState'
import { Page } from '@/components/layout/Page'
import {
  Alert,
  AlertAction,
  AlertContent,
  AlertDescription,
} from '@/components/motion/alert'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Skeleton } from '@/components/ui/skeleton'
import type { DescribedStep } from '@/lib/step-sentences'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  useAgentSettings,
  useUpdateAgentSettings,
} from '@/modules/api/agent-settings.hooks'
import { toastApiError } from '@/modules/toast/toast'
import { PagesVisited } from './PagesVisited'
import { ReplayTheater } from './ReplayTheater'
import { SnapshotGallery } from './SnapshotGallery'
import { TaskPageHeader } from './TaskPageHeader'
import { TaskPlayer } from './TaskPlayer'
import { TaskSteps } from './TaskSteps'
import { TechnicalDetails } from './TechnicalDetails'
import { useTaskDetailScreenData } from './task-detail.data'
import {
  actionForStep,
  activeStepIndex,
  dispatchTimes,
  snapshotForStep,
} from './task-detail.helpers'

/**
 * One page per task at `/audit/:sessionId`: what it was and how it ended, the
 * recording beside the steps it took in plain sentences, the pages it went to,
 * and technical details for support. Choosing a step moves the player there.
 */
export function TaskDetailPage({ focus }: { focus?: 'player' } = {}) {
  const { sessionId = '' } = useParams()
  // The replay link opens this page at the player, once, not on every poll.
  const focusedRef = useRef(false)
  const data = useTaskDetailScreenData(sessionId)
  const [showHidden, setShowHidden] = useState(false)
  // Without a recording, the step the person picked; null shows the last one.
  const [chosenDispatchId, setChosenDispatchId] = useState<number | null>(null)
  const [fullScreen, setFullScreen] = useState(false)

  if (data.isPending) {
    return (
      <Page>
        <Skeleton className="h-5 w-24" />
        <div className="flex flex-col gap-3">
          <Skeleton className="h-6 w-64" />
          <Skeleton className="h-9 w-3/4" />
          <Skeleton className="h-5 w-2/3" />
        </div>
        <div className="grid grid-cols-1 gap-5 lg:grid-cols-[minmax(0,1.65fr)_minmax(0,1fr)]">
          <Skeleton className="h-[420px] rounded-2xl" />
          <Skeleton className="h-[420px] rounded-2xl" />
        </div>
      </Page>
    )
  }
  if (data.isError || !data.detail) {
    return (
      <Page>
        <EmptyState
          title="This task isn’t here"
          hint="It may have been cleaned up, or the link is wrong. Your other tasks are in History."
        />
      </Page>
    )
  }

  const { session, dispatches } = data.detail
  const { replay } = data
  const steps = showHidden ? data.allSteps : data.steps
  const hasRecording = replay.plan.durationSeconds > 0
  // A recording still downloading already drives the steps from the start, so
  // the highlighted step does not jump when playback becomes ready.
  const followsPlayer = hasRecording || replay.recordingState === 'loading'

  const currentDispatchId = replay.state.currentAction?.frame.dispatchId
  const activeIndex = followsPlayer
    ? activeStepIndex(
        steps,
        currentDispatchId != null
          ? (dispatchTimes(dispatches).get(currentDispatchId) ?? null)
          : null,
      )
    : chosenIndex(steps, chosenDispatchId)
  const shownStep = steps[activeIndex]

  const selectStep = (step: DescribedStep) => {
    const action = actionForStep(step, replay.plan.actions)
    if (hasRecording && action) replay.selectAction(action)
    else setChosenDispatchId(step.dispatchIds[0] ?? null)
  }

  // Before the first step, a loading recording shows the first step's picture.
  const pictureStep = shownStep ?? steps[0]

  // One step list, fed once: the page's rail and the full-screen one stay in step.
  const stepList: ComponentProps<typeof TaskSteps> = {
    steps,
    activeIndex,
    startedAt: session.startedAt,
    hiddenCount: data.hiddenCount,
    showHidden,
    onShowHiddenChange: setShowHidden,
    onSelect: selectStep,
  }

  return (
    <Page>
      <TaskPageHeader session={session} lastPageUrl={data.lastPageUrl} />
      {session.status === 'failed' && !data.humanHelpEnabled ? (
        <AskForHelpTip />
      ) : null}
      <div className="grid grid-cols-1 items-stretch gap-5 lg:grid-cols-[minmax(0,1.65fr)_minmax(0,1fr)]">
        <TaskPlayer
          ref={(node) => {
            if (!node || focus !== 'player' || focusedRef.current) return
            focusedRef.current = true
            node.scrollIntoView({ block: 'start' })
          }}
          sessionId={sessionId}
          replay={replay}
          step={shownStep}
          snapshotId={
            pictureStep
              ? snapshotForStep(pictureStep, dispatches, data.screenshots)
              : null
          }
          onFullScreen={() => setFullScreen(true)}
        />
        <TaskSteps {...stepList} />
      </div>
      {hasRecording ? (
        <ReplayTheater
          open={fullScreen}
          onOpenChange={setFullScreen}
          session={session}
          replay={replay}
          steps={<TaskSteps {...stepList} layout="rail" />}
        />
      ) : null}
      <PagesVisited pages={data.pages} />
      <SnapshotGallery
        sessionId={sessionId}
        startedAt={session.startedAt}
        screenshots={data.screenshots}
        dispatches={dispatches}
        onPlayFrom={(dispatchId) => {
          const step = data.allSteps.find((candidate) =>
            candidate.dispatchIds.includes(dispatchId),
          )
          if (step) selectStep(step)
          document
            .getElementById('player')
            ?.scrollIntoView({ behavior: 'smooth', block: 'start' })
        }}
      />
      <TechnicalDetails session={session} dispatchCount={dispatches.length}>
        <StepLogDialog session={session} dispatches={dispatches} />
      </TechnicalDetails>
    </Page>
  )
}

// ---------------------------------------------------------------------------

function chosenIndex(
  steps: DescribedStep[],
  dispatchId: number | null,
): number {
  if (dispatchId !== null) {
    const index = steps.findIndex((step) =>
      step.dispatchIds.includes(dispatchId),
    )
    if (index !== -1) return index
  }
  return steps.length - 1
}

/** A failed task's one next step: let the agents ask for help next time. */
function AskForHelpTip() {
  const queryClient = useQueryClient()
  const update = useUpdateAgentSettings()

  const turnOn = () =>
    update.mutate(
      { humanHelpEnabled: true },
      {
        onSuccess: (saved) => {
          queryClient.setQueryData(useAgentSettings.getKey(), saved)
          track(AnalyticsEvent.HumanHelpToggled, {
            enabled: saved.humanHelpEnabled,
          })
        },
        onError: (error) =>
          toastApiError('Couldn’t turn that on. Try again in Settings.', error),
      },
    )

  return (
    <Alert
      variant="info"
      className="rounded-2xl px-5 py-4"
      contentClassName="flex-wrap items-center gap-3.5 md:flex-nowrap"
    >
      <Lightbulb aria-hidden className="size-5 shrink-0" />
      <AlertContent className="min-w-0 space-y-0">
        <AlertDescription className="text-[15px] text-ink">
          Next time, your AI agents can ask you for help instead of stopping,
          for example when a page asks you to sign in.
        </AlertDescription>
      </AlertContent>
      <AlertAction className="shrink-0 pt-0">
        <StatefulButton
          size="sm"
          state={
            update.isPending ? 'loading' : update.isSuccess ? 'success' : 'idle'
          }
          loadingText="Turning on"
          successText="Turned on"
          onClick={turnOn}
          className="h-[34px] px-3.5 font-semibold text-[13.5px]"
        >
          Turn on “Ask me when it gets stuck”
        </StatefulButton>
      </AlertAction>
    </Alert>
  )
}
