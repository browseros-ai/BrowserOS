import {
  Check,
  Compass,
  Eye,
  Hand,
  type LucideIcon,
  MousePointerClick,
  Paperclip,
  Send,
  TriangleAlert,
  Type,
} from 'lucide-react'
import { useReducedMotion } from 'motion/react'
import { useEffect, useRef } from 'react'
import { Button } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import type { DescribedStep } from '@/lib/step-sentences'
import { cn } from '@/lib/utils'
import type { ReplayVerb } from '@/modules/api/replay.hooks'
import { offsetLabel } from './task-detail.helpers'

interface TaskStepsProps {
  steps: DescribedStep[]
  /** Index into `steps` of the step playing or chosen, or -1 before the first. */
  activeIndex: number
  startedAt: number
  hiddenCount: number
  showHidden: boolean
  onShowHiddenChange: (show: boolean) => void
  onSelect: (step: DescribedStep) => void
}

const VERB_ICON: Record<ReplayVerb, LucideIcon> = {
  navigate: Compass,
  read: Eye,
  click: MousePointerClick,
  type: Type,
  attach: Paperclip,
  submit: Send,
  done: Check,
}

/**
 * "What happened": every step as a sentence on a rail, played steps in blue
 * and the current one filled. Choosing a step moves the player there.
 */
export function TaskSteps({
  steps,
  activeIndex,
  startedAt,
  hiddenCount,
  showHidden,
  onShowHiddenChange,
  onSelect,
}: TaskStepsProps) {
  const reduce = useReducedMotion() ?? false
  const scrollRef = useRef<HTMLDivElement>(null)

  // Scroll position is DOM state: keep the current step centred in the list's
  // own scroll area, so following playback never scrolls the page.
  useEffect(() => {
    const viewport = scrollRef.current
    const row = viewport?.querySelector<HTMLElement>(
      `[data-step-index="${activeIndex}"]`,
    )
    if (!viewport || !row) return
    viewport.scrollTo({
      top: row.offsetTop - viewport.clientHeight / 2 + row.offsetHeight / 2,
      behavior: reduce ? 'auto' : 'smooth',
    })
  }, [activeIndex, reduce])

  const progress =
    activeIndex >= 0
      ? `Step ${activeIndex + 1} of ${steps.length}`
      : `${steps.length} ${steps.length === 1 ? 'step' : 'steps'}`

  return (
    // contain: size lets the player set the row's height; the list then
    // scrolls inside it instead of stretching the row.
    <Card
      aria-labelledby="steps-title"
      size="compact"
      className="h-[460px] min-h-0 gap-1.5 overflow-hidden border-border-2 bg-card px-[18px] pt-[18px] pb-0 shadow-card lg:h-auto lg:min-h-[380px] lg:[contain:size]"
    >
      <div className="flex items-baseline justify-between gap-3">
        <h2
          id="steps-title"
          className="font-bold text-[15.5px] text-cyanotype-ink"
        >
          What happened
        </h2>
        <span className="text-[13px] text-ink-3 tabular-nums">{progress}</span>
      </div>
      <div
        ref={scrollRef}
        className="relative -mx-2.5 min-h-0 flex-1 overflow-y-auto overscroll-contain px-2.5 pt-1 pb-3 [mask-image:linear-gradient(to_bottom,transparent_0,#000_12px,#000_calc(100%-16px),transparent_100%)]"
      >
        {steps.length === 0 ? (
          <p className="px-2 py-3 text-[14px] text-ink-3">
            No steps were recorded for this task.
          </p>
        ) : (
          <ol className="flex flex-col">
            {steps.map((step, index) => (
              <StepRow
                key={step.dispatchIds[0] ?? index}
                step={step}
                index={index}
                activeIndex={activeIndex}
                last={index === steps.length - 1}
                time={offsetLabel(step.firstAt, startedAt)}
                onSelect={onSelect}
              />
            ))}
          </ol>
        )}
      </div>
      {hiddenCount > 0 ? (
        <div className="flex shrink-0 items-center justify-between gap-3 border-border border-t px-2 pt-2.5 pb-3.5 text-[13.5px] text-ink-3">
          <span>
            {hiddenCount} behind-the-scenes{' '}
            {hiddenCount === 1 ? 'step is' : 'steps are'}{' '}
            {showHidden ? 'shown in grey.' : 'hidden.'}
          </span>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onShowHiddenChange(!showHidden)}
            className="h-8 shrink-0 font-semibold text-[13px]"
          >
            {showHidden ? 'Hide them' : 'Show them'}
          </Button>
        </div>
      ) : null}
    </Card>
  )
}

// ---------------------------------------------------------------------------

function StepRow({
  step,
  index,
  activeIndex,
  last,
  time,
  onSelect,
}: {
  step: DescribedStep
  index: number
  activeIndex: number
  last: boolean
  time: string
  onSelect: (step: DescribedStep) => void
}) {
  const past = index < activeIndex
  const current = index === activeIndex
  const failed = step.isError
  const Icon = failed
    ? TriangleAlert
    : step.cancelled
      ? Hand
      : VERB_ICON[step.verb]

  return (
    <li className="relative">
      {last ? null : (
        <span
          aria-hidden
          className={cn(
            'absolute top-[42px] -bottom-2 left-[22px] w-0.5 rounded-full',
            past ? 'bg-accent-tint-2' : 'bg-border',
          )}
        />
      )}
      <button
        type="button"
        data-step-index={index}
        aria-current={current ? 'step' : undefined}
        onClick={() => onSelect(step)}
        className={cn(
          'grid w-full grid-cols-[30px_minmax(0,1fr)_auto] items-start gap-3 rounded-xl px-2 py-2.5 text-left outline-none transition-colors duration-200 focus-visible:ring-2 focus-visible:ring-ring',
          current ? 'bg-accent-tint' : 'hover:bg-bg-sunken',
        )}
      >
        <span
          aria-hidden
          className={cn(
            'grid size-[30px] place-items-center rounded-full border transition-colors duration-200',
            failed
              ? 'border-transparent bg-red-tint text-red-ink'
              : current
                ? 'border-transparent bg-primary text-primary-foreground'
                : past
                  ? 'border-transparent bg-accent-tint text-accent-ink'
                  : 'border-border bg-bg-sunken text-ink-2',
            step.hidden && !current && 'border-dashed',
          )}
        >
          <Icon className="size-4" strokeWidth={1.75} />
        </span>
        <span
          className={cn(
            'pt-1 text-[14.5px] leading-[1.4]',
            current || (past && !step.hidden) ? 'text-ink' : 'text-ink-3',
          )}
        >
          {step.sentence}
        </span>
        <span className="pt-1.5 text-[12.5px] text-ink-4 tabular-nums">
          {time}
        </span>
      </button>
    </li>
  )
}
