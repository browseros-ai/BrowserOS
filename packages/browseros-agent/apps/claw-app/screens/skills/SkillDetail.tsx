/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill, SkillRun } from '@browseros/claw-api'
import { skipToken } from '@tanstack/react-query'
import {
  Check,
  ChevronDown,
  CirclePause,
  Eye,
  FileText,
  ListChecks,
  LoaderCircle,
  Lock,
  MessageCircle,
  MessageCircleQuestion,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  TriangleAlert,
  UserRoundCheck,
  Wrench,
} from 'lucide-react'
import {
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { Link, NavLink, useNavigate } from 'react-router'
import { CodeBlock } from '@/components/agents/code-block'
import { SessionStatusBadge } from '@/components/audit/SessionStatusBadge'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { AgentMark } from '@/components/harness/AgentMark'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { CARD_TARGET_CLASS, Card } from '@/components/motion/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'
import { SkillPicture } from '@/components/pro/UltrafastPictures'
import { NowUltrafastStrip } from '@/components/skills/NowUltrafast'
import { RunsChoiceDialog } from '@/components/skills/RunsChoiceDialog'
import { SiteBadge } from '@/components/skills/SiteBadge'
import { SkillActions } from '@/components/skills/SkillActions'
import { SkillValue } from '@/components/skills/SkillValue'
import { UltrafastNudge } from '@/components/skills/UltrafastNudge'
import { UltrafastState } from '@/components/skills/UltrafastState'
import { UseSkill } from '@/components/skills/UseSkill'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  isCapturing,
  subscribeToCaptureState,
} from '@/modules/analytics/posthog'
import { useSkillFlow } from '@/modules/api/skills.hooks'
import { taskDayAndTime } from '@/screens/audit/audit.helpers'
import { useSkillDetailData } from './skill-detail.data'
import {
  awaitsFirstRun,
  chosenLine,
  firstRunLearning,
  healthLine,
  type LearningStep,
  normalRunLabel,
  parseSkillBody,
  type RunsChoice,
  runModeCounts,
  runModeLabel,
  runsChoice,
  skillValue,
  stuckWhile,
  ultrafastStateToken,
  usageLevel,
} from './skills.helpers'

const BACK = { to: '/skills', label: 'Skills' }

/**
 * One skill, built around using it: the phrase to copy and paste into an agent leads, then what
 * the skill has done for this person and anything that needs a word. Steps, how it runs, runs and
 * the file wait in one closed fold.
 */
export function SkillDetail() {
  const { detail, agents, isLoading, isError, checking, nudge } =
    useSkillDetailData()
  const navigate = useNavigate()
  const viewed = useRef<string | null>(null)
  const viewedSkill = detail?.skill
  const capturing = useSyncExternalStore(
    subscribeToCaptureState,
    isCapturing,
    () => false,
  )
  // Counting a page view is a side effect of the page showing a skill; no handler runs then.
  // The ref keeps a refetch of the same skill from counting twice, and is only set once
  // analytics is capturing, so a deep link opened before it starts still counts.
  useEffect(() => {
    if (!capturing || !viewedSkill || viewed.current === viewedSkill.name) {
      return
    }
    viewed.current = viewedSkill.name
    track(AnalyticsEvent.SkillPageViewed, {
      usage_level: usageLevel(viewedSkill.runCount),
      ultrafast_state: ultrafastStateToken(viewedSkill.ultrafast),
    })
  }, [capturing, viewedSkill])

  if (isError) {
    return (
      <Page>
        <PageHeader back={BACK} title="This skill isn’t here" />
        <Card
          size="compact"
          className="flex-row items-center gap-4 border-border-2 bg-card px-[22px] py-5 shadow-card"
        >
          <span
            aria-hidden
            className="grid size-11 shrink-0 place-items-center rounded-full bg-red-tint text-red-ink"
          >
            <TriangleAlert className="size-5" />
          </span>
          <span className="text-[14.5px] text-ink-2">
            It may have been deleted, or BrowserOS isn’t running. Your other
            skills are in the list.
          </span>
        </Card>
      </Page>
    )
  }
  if (isLoading || !detail) return <DetailSkeleton />

  const { skill, body, runs } = detail
  const learning = firstRunLearning(skill, checking)
  const choice = runsChoice(skill.ultrafast, runs)
  const urgent = choice?.state === 'choose' || choice?.state === 'reask'

  return (
    <Page className="gap-6 md:gap-7">
      <PageHeader
        back={BACK}
        eyebrow={
          <>
            <SiteBadge site={skill.site} className="size-8" />
            <span className="text-[14px] text-ink-3">
              {skill.site ?? 'Any website'}
            </span>
          </>
        }
        title={skill.description}
        lede={
          <UltrafastState
            ultrafast={skill.ultrafast}
            afterFirstRun={awaitsFirstRun(skill)}
            wordmarkClassName="text-[14.5px]"
            className="text-[14px]"
          />
        }
        actions={
          <SkillActions
            skill={skill}
            body={body}
            onDeleted={() => navigate('/skills')}
            menuClassName="rounded-[10px] border border-border-2 bg-card shadow-card"
          />
        }
      />

      <UseSkill name={skill.name} agents={agents}>
        {nudge ? (
          <UltrafastNudge surface="skill_page" className="text-[14px]">
            <SpeedNote normal={normalRunLabel(runs)} />
          </UltrafastNudge>
        ) : null}
      </UseSkill>

      <NowUltrafastStrip skill={skill} />
      {learning ? (
        <LearningPanel steps={learning} reason={skill.ultrafast?.reason} />
      ) : null}
      <StateNotice skill={skill} />
      {urgent && choice ? (
        <Card
          size="compact"
          className="gap-0 border-border-2 bg-card px-5 py-[18px] text-[14.5px] text-ink-2 shadow-card"
        >
          <RunsChoiceBlock skill={skill} choice={choice} />
        </Card>
      ) : null}
      {learning ? null : (
        <SkillValue value={skillValue(skill, runs, Date.now())} />
      )}

      <MoreAboutSkill>
        <div className="flex flex-col gap-5">
          <FoldPart title="Steps neo follows">
            <StepsList body={body} />
          </FoldPart>
          <FoldPart title="Things neo learned">
            <LearnedNotes body={body} />
          </FoldPart>
          <FoldPart title="Works in">
            <WorksIn skill={skill} />
          </FoldPart>
          <SkillFile body={body} />
        </div>
        <div className="flex flex-col gap-5">
          <FoldPart title="How it runs">
            <UltrafastSection
              skill={skill}
              runs={runs}
              learning={learning !== null}
              urgent={urgent}
            />
          </FoldPart>
          <FoldPart title="Recent runs">
            <RecentRuns runs={runs} />
          </FoldPart>
        </div>
      </MoreAboutSkill>
    </Page>
  )
}

// ---------------------------------------------------------------------------

/** "Runs at normal speed, about 41 sec a run. With ultrafast, neo runs it in seconds." */
function SpeedNote({ normal }: { normal: string | null }) {
  return (
    <>
      Runs at normal speed{normal ? `, ${normal} a run` : ''}. With{' '}
      <UltrafastWordmark />, neo runs it in seconds.
    </>
  )
}

/** The closed fold that holds everything but the copy box and the skill's value. */
function MoreAboutSkill({ children }: { children: ReactNode }) {
  return (
    <Collapsible className="rounded-[18px] border border-border-2 bg-card shadow-card">
      <CollapsibleTrigger className="flex w-full items-center gap-3 rounded-[18px] px-[18px] py-3.5 text-left outline-none hover:bg-bg-canvas focus-visible:ring-2 focus-visible:ring-ring">
        <span
          aria-hidden
          className="grid size-8 shrink-0 place-items-center rounded-[9px] bg-accent-tint text-accent-ink"
        >
          <ListChecks className="size-4" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="font-bold text-[15px] text-cyanotype-ink">
            More about this skill
          </span>
          <span className="text-[13px] text-ink-3">
            Steps, how it runs, recent runs, the file
          </span>
        </span>
        <CollapsibleIndicator>
          <ChevronDown aria-hidden className="size-4 text-ink-3" />
        </CollapsibleIndicator>
      </CollapsibleTrigger>
      <CollapsibleContent contentClassName="grid grid-cols-1 gap-x-7 gap-y-5 border-border-2 border-t px-[18px] pt-4 pb-5 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
        {children}
      </CollapsibleContent>
    </Collapsible>
  )
}

function FoldPart({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5">
      <h2 className="font-bold text-[13.5px] text-cyanotype-ink">{title}</h2>
      {children}
    </section>
  )
}

function StepsList({ body }: { body: string }) {
  const { steps } = parseSkillBody(body)
  if (!steps.length) {
    return (
      <p className="text-[14px] text-ink-2">
        The steps appear here after the first run.
      </p>
    )
  }
  return (
    <ol className="flex flex-col gap-2.5">
      {steps.map((step, index) => (
        <li
          key={step}
          className="grid grid-cols-[24px_minmax(0,1fr)] items-start gap-2.5 text-[14px] text-ink-2"
        >
          <span className="grid size-6 place-items-center rounded-lg bg-accent-tint font-bold text-[12px] text-accent-ink tabular-nums">
            {index + 1}
          </span>
          <span className="pt-0.5">{step}</span>
        </li>
      ))}
    </ol>
  )
}

function LearnedNotes({ body }: { body: string }) {
  const { learnedNotes } = parseSkillBody(body)
  if (!learnedNotes.length) {
    return (
      <p className="text-[14px] text-ink-2">
        Nothing yet. neo adds notes as it learns this site.
      </p>
    )
  }
  return (
    <ul className="flex list-disc flex-col gap-1.5 pl-[18px] text-[14px] text-ink-2 marker:text-ink-4">
      {learnedNotes.map((note) => (
        <li key={note}>{note}</li>
      ))}
    </ul>
  )
}

/** The SKILL.md itself, behind its own toggle inside the fold. */
function SkillFile({ body }: { body: string }) {
  return (
    <Collapsible>
      <CollapsibleTrigger className="flex items-center gap-2 font-semibold text-[13.5px] text-ink-2 outline-none hover:text-cyanotype-ink focus-visible:ring-2 focus-visible:ring-ring">
        <FileText aria-hidden className="size-[15px]" />
        <span>View as a file</span>
        <CollapsibleIndicator>
          <ChevronDown aria-hidden className="size-3.5" />
        </CollapsibleIndicator>
      </CollapsibleTrigger>
      <CollapsibleContent contentClassName="pt-2.5">
        <CodeBlock code={body} language="text" maxHeight={420} copyable />
      </CollapsibleContent>
    </Collapsible>
  )
}

/** A state that needs a word above the fold: a fast version that needs a repair or is paused. */
function StateNotice({ skill }: { skill: Skill }) {
  const ultrafast = skill.ultrafast
  if (!ultrafast || ultrafast.locked) return null
  if (ultrafast.status === 'degraded') {
    return (
      <Note tone="amber" icon={<Wrench aria-hidden className="size-4" />}>
        <strong className="text-cyanotype-ink">
          ultrafast needs a repair.
        </strong>{' '}
        {ULTRAFAST_STATE.degraded}
      </Note>
    )
  }
  if (ultrafast.status === 'broken') {
    return (
      <Note tone="red" icon={<CirclePause aria-hidden className="size-4" />}>
        <strong className="text-cyanotype-ink">ultrafast is paused.</strong>{' '}
        {ULTRAFAST_STATE.broken}
      </Note>
    )
  }
  return null
}

function WorksIn({ skill }: { skill: Skill }) {
  if (!skill.linkedAgents.length) {
    return (
      <p className="text-[14px] text-ink-2">
        None of your AI agents have it yet. Connect one and save the task again.
      </p>
    )
  }
  return (
    <div className="flex flex-wrap gap-2">
      {skill.linkedAgents.map((agent) => (
        <AgentMark
          key={agent}
          harness={agent}
          className="rounded-md border border-border-2 bg-bg-sunken px-3 py-1.5 font-semibold text-[13.5px] text-ink-2"
        />
      ))}
    </div>
  )
}

/**
 * How the skill runs, inside the fold: its fast version's state, how runs have gone, and whether it
 * runs with the agent. A choice the skill still needs is shown at the top of the page instead, and
 * a first run being learned points to its panel there.
 */
function UltrafastSection({
  skill,
  runs,
  learning,
  urgent,
}: {
  skill: Skill
  runs: SkillRun[]
  /** The learning panel above already says what is happening. */
  learning: boolean
  /** The choice is shown above the fold. */
  urgent: boolean
}) {
  const ultrafast = skill.ultrafast
  if (!ultrafast || ultrafast.status === 'none') {
    return (
      <p className="text-[14px] text-ink-2">
        {ultrafast?.locked
          ? 'Runs at normal speed.'
          : awaitsFirstRun(skill)
            ? 'After its first run, neo runs this skill the ultrafast way.'
            : 'This skill runs with your agent.'}
      </p>
    )
  }
  if (learning) {
    return (
      <p className="text-[14px] text-ink-2">
        neo is learning this skill. See the steps at the top of this page.
      </p>
    )
  }
  const counts = runModeCounts(runs)
  const health = ultrafast.locked ? '' : healthLine(ultrafast, Date.now())
  const ranFast = counts.ultrafast + counts.handedBack > 0
  const choice = runsChoice(ultrafast, runs)
  return (
    <div className="overflow-hidden rounded-2xl border border-border-2 bg-card">
      <div className="relative flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1.5 border-accent-tint-2 border-b bg-accent-tint py-3.5 pr-4 pl-5 before:absolute before:inset-y-0 before:left-0 before:w-[3px] before:bg-primary dark:border-primary/25 dark:bg-primary/12">
        <h3>
          {ultrafast.locked ? (
            <span className="inline-flex items-baseline gap-1.5 text-[14px]">
              <UltrafastWordmark className="text-[17px] text-ink-3" />
              <span className="font-semibold text-ink-3">off</span>
            </span>
          ) : (
            <UltrafastState
              ultrafast={ultrafast}
              wordmarkClassName="text-[17px]"
              className="text-[14px]"
            />
          )}
        </h3>
        {health ? <p className="text-[13px] text-ink-2">{health}</p> : null}
      </div>
      <div className="flex flex-col gap-2.5 px-4 pt-4 pb-4 text-[14px] text-ink-2">
        {urgent ? (
          <p>You can choose how it runs at the top of this page.</p>
        ) : choice ? null : (
          <p>{ULTRAFAST_STATE[ultrafast.status]}</p>
        )}
        {ultrafast.reason ? (
          <p className="text-[13.5px] text-ink-3">{ultrafast.reason}</p>
        ) : null}
        {ranFast ? (
          <p className="text-[13.5px] text-ink-3">
            {counts.ultrafast} ultrafast · {counts.handedBack} handed back ·{' '}
            {counts.normal} the normal way
          </p>
        ) : null}
        {choice && !urgent ? (
          <RunsChoiceBlock skill={skill} choice={choice} />
        ) : null}
        {ultrafast.flowVersion > 0 && !ultrafast.locked ? (
          <FlowFile name={skill.name} />
        ) : null}
      </div>
    </div>
  )
}

const LEARNING_ICON: Record<LearningStep['state'], ReactNode> = {
  done: <Check />,
  now: <LoaderCircle className="motion-safe:animate-spin" />,
  todo: <Sparkles />,
}

const LEARNING_ICON_CLASS: Record<LearningStep['state'], string> = {
  done: 'bg-green-tint text-green-ink',
  now: 'bg-accent-tint text-accent-ink',
  todo: 'bg-bg-sunken text-ink-4',
}

const LEARNING_STATE_WORDS: Record<LearningStep['state'], string> = {
  done: 'Done:',
  now: 'In progress:',
  todo: 'Still to come:',
}

/** The first run of a task skill, in plain words, while neo turns it into its fast version. */
function LearningPanel({
  steps,
  reason,
}: {
  steps: LearningStep[]
  reason?: string
}) {
  return (
    <Card
      size="compact"
      className="gap-0 overflow-hidden border-border-2 bg-card p-0 shadow-card"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1.5 border-border-2 border-b bg-bg-sunken px-5 py-4">
        <h2>
          <UltrafastWordmark className="text-[19px] text-ink-3" />
        </h2>
        <p className="text-[13.5px] text-ink-2">Learning from this run</p>
      </div>
      <div className="flex flex-col gap-3 px-5 pt-[18px] pb-5">
        <ul className="flex flex-col gap-2.5 text-[14.5px]">
          {steps.map((step) => (
            <li
              key={step.text}
              className={cn(
                'flex items-start gap-2.5',
                step.state === 'todo' ? 'text-ink-3' : 'text-ink',
              )}
            >
              <span
                aria-hidden
                className={cn(
                  'grid size-[22px] shrink-0 place-items-center rounded-full [&_svg]:size-[13px]',
                  LEARNING_ICON_CLASS[step.state],
                )}
              >
                {LEARNING_ICON[step.state]}
              </span>
              <span className="pt-px">
                <span className="sr-only">
                  {LEARNING_STATE_WORDS[step.state]}{' '}
                </span>
                {step.text}
              </span>
            </li>
          ))}
        </ul>
        {reason ? <p className="text-[13.5px] text-ink-3">{reason}</p> : null}
      </div>
    </Card>
  )
}

const ALWAYS_WITH_REASON: Record<string, string> = {
  pay: 'This skill pays for something.',
  order: 'This skill places an order.',
  send: 'This skill sends something for you.',
  delete: 'This skill deletes something.',
}

/** "Step by step" or "In one go" for a skill that changes something. */
function RunsChoiceBlock({
  skill,
  choice,
}: {
  skill: Skill
  choice: RunsChoice
}) {
  const [target, setTarget] = useState<'with_agent' | 'without_agent' | null>(
    null,
  )
  const detail = skill.ultrafast?.runs
  const chip =
    choice.chipTone === 'ultrafast' ? (
      <p className="font-semibold text-[14px] text-cyanotype-ink">
        <UltrafastWordmark className="mr-1.5" />
        {choice.chip}
      </p>
    ) : choice.chipTone === 'amber' ? (
      <p className="flex items-center gap-2 font-bold text-[14px] text-amber-ink">
        {choice.state === 'choose' ? (
          <MessageCircleQuestion aria-hidden className="size-[15px]" />
        ) : null}
        {choice.chip}
      </p>
    ) : (
      <p className="font-semibold text-[14px] text-ink-2">{choice.chip}</p>
    )
  if (choice.state === 'read') {
    return (
      <div className="flex flex-col gap-2.5">
        <Note icon={<Eye aria-hidden className="size-4" />}>
          This skill only reads pages, so your agent always runs it in one go.
          There’s nothing to choose.
        </Note>
      </div>
    )
  }
  const click = detail?.click
  const site = detail?.site
  const chosen = detail ? chosenLine(detail, Date.now()) : null
  return (
    <div className="flex flex-col gap-3">
      {chip}
      {choice.state === 'reask' ? (
        <Note tone="amber" icon={<RefreshCw aria-hidden className="size-4" />}>
          <strong className="text-cyanotype-ink">The last step changed.</strong>{' '}
          It now does this{site ? ` on ${site}` : ''}: “{click}”. Your earlier
          choice was for a different step, so it runs step by step until you
          choose again.
        </Note>
      ) : null}
      {choice.state === 'always' && detail?.alwaysWithAgent ? (
        <p>{ALWAYS_WITH_REASON[detail.alwaysWithAgent]}</p>
      ) : null}
      {choice.state === 'locked' ? (
        <p>
          ultrafast is off on this computer, so this skill runs step by step
          now.
        </p>
      ) : null}
      {choice.state !== 'reask' &&
      choice.state !== 'always' &&
      choice.state !== 'locked' &&
      click ? (
        <p>
          This skill changes something{site ? ` on ${site}` : ''}. Its last
          step: “{click}”
        </p>
      ) : null}
      <RadioGroup
        aria-label="How it runs when you ask for it"
        value={choice.selected}
        disabled={choice.disabled}
        onValueChange={(value) => {
          if (value === 'with_agent' || value === 'without_agent') {
            if (value !== choice.selected) setTarget(value)
          }
        }}
        className="gap-2.5"
      >
        <p className="text-[13.5px] text-ink-3">
          How it runs when you ask for it
        </p>
        <RunsOption
          value="with_agent"
          selected={choice.selected === 'with_agent'}
          disabled={choice.disabled}
          icon={<UserRoundCheck aria-hidden className="size-4" />}
          title="Step by step"
          time={choice.slowLabel}
          why="Your agent reads each page as it goes, so it can stop if something looks wrong. Safer."
        />
        <RunsOption
          value="without_agent"
          selected={choice.selected === 'without_agent'}
          disabled={choice.withoutLocked}
          icon={
            choice.withoutLocked ? (
              <Lock aria-hidden className="size-4" />
            ) : null
          }
          ultrafast
          title="In one go"
          time={choice.fastLabel}
          why="Your agent hands it to neo, which runs every step. It only stops if a step can’t find what it expects."
        />
      </RadioGroup>
      {choice.state === 'choose' ? (
        <Note
          tone="amber"
          icon={<MessageCircleQuestion aria-hidden className="size-4" />}
        >
          You haven’t chosen yet, so it runs step by step. Your agent will ask
          you the next time you use it, or you can choose here.
        </Note>
      ) : null}
      {choice.state === 'reask' ? (
        <p className="text-[13.5px] text-ink-3">
          Your agent will ask you the next time you use it.
        </p>
      ) : null}
      {(choice.state === 'without' || choice.state === 'with') && chosen ? (
        <Note icon={<MessageCircle aria-hidden className="size-4" />}>
          {chosen}
          {detail?.answer ? (
            <>
              :{' '}
              <span className="font-semibold text-cyanotype-ink">
                “{detail.answer}”
              </span>
            </>
          ) : (
            '.'
          )}
        </Note>
      ) : null}
      {choice.state === 'always' ? (
        <Note icon={<ShieldCheck aria-hidden className="size-4" />}>
          Skills that pay, place an order, send a message or delete something
          always run step by step, so your agent can check each page first.
        </Note>
      ) : null}
      {choice.state === 'locked' ? (
        <div className="max-w-sm">
          <SkillPicture />
        </div>
      ) : null}
      {choice.state === 'locked' ? (
        <Link
          to="/pro"
          className="self-start rounded-lg bg-primary px-3 py-1.5 font-semibold text-[13.5px] text-primary-foreground outline-none hover:opacity-90 focus-visible:ring-2 focus-visible:ring-ring"
        >
          See ultrafast
        </Link>
      ) : null}
      <RunsChoiceDialog
        name={skill.name}
        target={target}
        click={click}
        site={site}
        fastLabel={choice.fastLabel}
        onClose={() => setTarget(null)}
      />
    </div>
  )
}

function RunsOption({
  value,
  selected,
  disabled,
  icon,
  ultrafast = false,
  title,
  time,
  why,
}: {
  value: 'with_agent' | 'without_agent'
  selected: boolean
  disabled: boolean
  icon: ReactNode
  /** The ultrafast way to run: a Signal Blue edge and the wordmark after the title. */
  ultrafast?: boolean
  title: string
  time: string | null
  why: string
}) {
  const titleId = useId()
  return (
    <Card
      size="compact"
      interactive={!disabled}
      selected={selected}
      className={cn(
        'relative flex-row gap-3 px-3.5 py-3',
        ultrafast &&
          'before:absolute before:-inset-y-px before:-left-px before:w-[3px] before:rounded-l-2xl before:bg-primary',
        disabled && 'opacity-55',
      )}
    >
      <RadioGroupItem
        value={value}
        disabled={disabled}
        aria-labelledby={titleId}
        className={`mt-0.5 ${CARD_TARGET_CLASS}`}
      />
      <span className="flex flex-col gap-0.5">
        <span className="flex items-center gap-1.5 font-bold text-[14.5px] text-cyanotype-ink">
          {icon}
          <span id={titleId}>{title}</span>{' '}
          {ultrafast ? <UltrafastWordmark className="text-[13.5px]" /> : null}
        </span>
        {time ? (
          <span
            className={cn(
              'font-bold text-[17px]',
              ultrafast ? 'text-accent-ink' : 'text-cyanotype-ink',
            )}
          >
            {time}
          </span>
        ) : null}
        <span className="text-[13.5px] text-ink-2 leading-snug">{why}</span>
      </span>
    </Card>
  )
}

const NOTE_BOX: Record<'plain' | 'amber' | 'red', string> = {
  plain: 'border border-border-2 bg-bg-sunken text-ink-2',
  amber: 'bg-amber-tint text-ink-2',
  red: 'bg-red-tint text-ink-2',
}

const NOTE_ICON: Record<'plain' | 'amber' | 'red', string> = {
  plain: 'text-ink-3',
  amber: 'text-amber-ink',
  red: 'text-red-ink',
}

function Note({
  icon,
  tone = 'plain',
  children,
}: {
  icon: ReactNode
  tone?: 'plain' | 'amber' | 'red'
  children: ReactNode
}) {
  const box = NOTE_BOX[tone]
  const iconColor = NOTE_ICON[tone]
  return (
    <div
      className={`flex items-start gap-2.5 rounded-xl px-3.5 py-3 text-[14px] ${box}`}
    >
      <span className={`mt-0.5 shrink-0 ${iconColor}`}>{icon}</span>
      <span>{children}</span>
    </div>
  )
}

const ULTRAFAST_STATE: Record<string, string> = {
  verified:
    'Your agent runs this skill in about a second. If the site changes, it stops and finishes the task the normal way.',
  degraded:
    'The site changed. Your agent finishes these runs the normal way and repairs the fast version as it goes.',
  broken:
    'The fast version kept failing, so it is paused. Your agent runs the skill the normal way and can make it fast again.',
  pending: 'Your agent is turning this skill into a fast version.',
  off: 'This skill runs the normal way.',
}

/** The flow file, fetched only once someone opens it. */
function FlowFile({ name }: { name: string }) {
  const [open, setOpen] = useState(false)
  const flow = useSkillFlow({ variables: open ? { name } : skipToken })
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="flex items-center gap-1.5 font-semibold text-[13.5px] text-ink-2 outline-none hover:text-cyanotype-ink focus-visible:ring-2 focus-visible:ring-ring">
        <span>See the fast version</span>
        <CollapsibleIndicator>
          <ChevronDown aria-hidden className="size-3.5" />
        </CollapsibleIndicator>
      </CollapsibleTrigger>
      <CollapsibleContent contentClassName="pt-2.5">
        {flow.isError ? (
          <p className="text-[13.5px] text-ink-3">Couldn’t load it.</p>
        ) : flow.isPending ? (
          <Skeleton className="h-24 rounded-lg" />
        ) : (
          <CodeBlock
            code={flow.data.source}
            language="text"
            maxHeight={320}
            copyable
          />
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}

function RecentRuns({ runs }: { runs: SkillRun[] }) {
  const now = Date.now()
  return (
    <>
      {runs.length ? (
        <ul className="-mx-2 flex flex-col">
          {runs.map((run) => (
            <li key={run.id}>
              <NavLink
                to={`/audit/${encodeURIComponent(run.sessionId)}`}
                className="flex items-start gap-3 rounded-md px-2 py-2.5 outline-none transition-colors hover:bg-bg-sunken focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="font-semibold text-[14px] text-cyanotype-ink">
                    {taskDayAndTime(run.createdAt, now)}
                  </span>
                  <AgentMark
                    label={run.agentId}
                    className="text-[13.5px] text-ink-3"
                  />
                  {run.mode === 'ultrafast' ? (
                    <UltrafastWordmark className="text-[13.5px]" />
                  ) : runModeLabel(run) ? (
                    <span className="text-[13.5px] text-ink-3">
                      {runModeLabel(run)}
                    </span>
                  ) : null}
                  {run.clean ? null : (
                    <span className="text-[13.5px] text-ink-3">
                      {stuckWhile(run.erroredTool)}
                    </span>
                  )}
                </span>
                <SessionStatusBadge status={run.clean ? 'done' : 'failed'} />
              </NavLink>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-[14px] text-ink-2">This skill hasn’t run yet.</p>
      )}
    </>
  )
}

function DetailSkeleton() {
  return (
    <Page>
      <div className="flex flex-col gap-3">
        <Skeleton className="h-5 w-28" />
        <Skeleton className="h-8 w-40" />
        <Skeleton className="h-9 w-3/4" />
        <Skeleton className="h-8 w-72" />
      </div>
      <Skeleton className="h-[136px] rounded-[22px]" />
      <Skeleton className="h-[84px] rounded-[18px]" />
      <Skeleton className="h-[66px] rounded-[18px]" />
    </Page>
  )
}
