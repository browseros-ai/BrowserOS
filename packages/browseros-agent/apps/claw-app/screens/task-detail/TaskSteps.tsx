import { type TodoItem, TodoList } from '@/components/agents/todo-list'
import { Button } from '@/components/motion/button/base'
import type { DescribedStep } from '@/lib/step-sentences'
import { offsetLabel } from './task-detail.helpers'

interface TaskStepsProps {
  steps: DescribedStep[]
  /** Index into `steps` of the step playing or chosen, or -1 before the first. */
  activeIndex: number
  startedAt: number
  /** Spins the current step's ring only while the recording plays. */
  playing: boolean
  hiddenCount: number
  showHidden: boolean
  onShowHiddenChange: (show: boolean) => void
  onSelect: (step: DescribedStep) => void
}

/** "What happened": every step as a sentence; choosing one moves the player there. */
export function TaskSteps({
  steps,
  activeIndex,
  startedAt,
  playing,
  hiddenCount,
  showHidden,
  onShowHiddenChange,
  onSelect,
}: TaskStepsProps) {
  const items: TodoItem[] = steps.map((step, index) => ({
    id: String(index),
    title: step.hidden ? (
      <span className="text-ink-3">{step.sentence}</span>
    ) : (
      step.sentence
    ),
    status:
      step.isError || step.cancelled
        ? 'cancelled'
        : index < activeIndex
          ? 'completed'
          : index === activeIndex
            ? 'in-progress'
            : 'pending',
    // A full ring marks the current step; the spin means it is playing.
    progress: index === activeIndex && !playing ? 100 : undefined,
    detail: (
      <span className="text-[12.5px] tabular-nums">
        {offsetLabel(step.firstAt, startedAt)}
      </span>
    ),
  }))

  return (
    // contain: size lets the player set the row's height; the list then
    // scrolls inside it instead of stretching the row.
    <div className="flex h-[460px] min-h-0 flex-col gap-2 lg:h-auto lg:min-h-[380px] lg:[contain:size]">
      <TodoList
        fill
        title="What happened"
        items={items}
        collapseOnComplete={false}
        strikeCompleted={false}
        onSelect={(id) => {
          const step = steps[Number(id)]
          if (step) onSelect(step)
        }}
        className="min-h-0 flex-1 border-border-2 bg-card shadow-card [&_h3]:font-bold [&_h3]:text-[15.5px] [&_h3]:text-cyanotype-ink"
      />
      {hiddenCount > 0 ? (
        <p className="flex flex-wrap items-center justify-between gap-2 px-1 text-[13.5px] text-ink-3">
          <span>
            {hiddenCount} behind-the-scenes{' '}
            {hiddenCount === 1 ? 'step is' : 'steps are'}{' '}
            {showHidden ? 'shown in grey.' : 'hidden.'}
          </span>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onShowHiddenChange(!showHidden)}
            className="h-8 font-semibold text-[13px]"
          >
            {showHidden ? 'Hide them' : 'Show them'}
          </Button>
        </p>
      ) : null}
    </div>
  )
}
