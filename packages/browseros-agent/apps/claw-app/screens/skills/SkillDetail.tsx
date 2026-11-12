/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill, SkillRun } from '@browseros/claw-api'
import { skipToken } from '@tanstack/react-query'
import {
  ChevronDown,
  Eye,
  FileText,
  Lock,
  MessageCircle,
  MessageCircleQuestion,
  RefreshCw,
  ShieldCheck,
  TriangleAlert,
  UserRoundCheck,
  Zap,
} from 'lucide-react'
import { type ReactNode, useId, useState } from 'react'
import { Link, NavLink, useNavigate } from 'react-router'
import { CodeBlock } from '@/components/agents/code-block'
import { SessionStatusBadge } from '@/components/audit/SessionStatusBadge'
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
import { Phrase } from '@/components/skills/Phrase'
import { RunSkillButton } from '@/components/skills/RunSkillButton'
import { RunsChoiceDialog } from '@/components/skills/RunsChoiceDialog'
import { SiteBadge } from '@/components/skills/SiteBadge'
import { SkillActions } from '@/components/skills/SkillActions'
import { UltrafastBadge } from '@/components/skills/UltrafastBadge'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { Skeleton } from '@/components/ui/skeleton'
import { useSkillFlow } from '@/modules/api/skills.hooks'
import { taskDayAndTime } from '@/screens/audit/audit.helpers'
import { useSkillDetailData } from './skill-detail.data'
import {
  chosenLine,
  healthLine,
  parseSkillBody,
  type RunsChoice,
  runModeCounts,
  runModeLabel,
  runsChoice,
  skillCommand,
  stuckWhile,
  usedLabel,
} from './skills.helpers'

const BACK = { to: '/skills', label: 'Skills' }

/** One skill: the steps neo follows, what it learned, and its runs. */
export function SkillDetail() {
  const { detail, isLoading, isError } = useSkillDetailData()
  const navigate = useNavigate()

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
  const { steps, learnedNotes } = parseSkillBody(body)

  return (
    <Page>
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
          <div className="flex flex-col gap-2.5 pt-1">
            <div className="flex flex-wrap items-center gap-2">
              <Phrase className="px-3 py-1.5 text-[14px]">
                {skillCommand(skill.name)}
              </Phrase>
              <RunSkillButton name={skill.name} />
              <SkillActions
                skill={skill}
                body={body}
                onDeleted={() => navigate('/skills')}
              />
            </div>
            <p className="flex flex-wrap items-center gap-2 text-[14px] text-ink-3">
              <UltrafastBadge ultrafast={skill.ultrafast} />
              {usedLabel(skill.runCount, skill.lastRunAt, Date.now())}
            </p>
          </div>
        }
      />

      <div className="grid grid-cols-1 items-start gap-5 lg:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)]">
        <div className="flex flex-col gap-5">
          <Section title="Steps neo follows">
            {steps.length ? (
              <ol className="flex flex-col gap-3">
                {steps.map((step, index) => (
                  <li
                    key={step}
                    className="grid grid-cols-[28px_minmax(0,1fr)] items-start gap-3 text-[14.5px] text-ink"
                  >
                    <span className="grid size-7 place-items-center rounded-full bg-accent-tint font-semibold text-[13px] text-accent-ink tabular-nums">
                      {index + 1}
                    </span>
                    <span className="pt-1">{step}</span>
                  </li>
                ))}
              </ol>
            ) : (
              <p className="text-[14.5px] text-ink-2">
                The steps appear here after the first run.
              </p>
            )}
          </Section>
          <Section title="Things neo learned">
            {learnedNotes.length ? (
              <ul className="flex list-disc flex-col gap-2 pl-5 text-[14.5px] text-ink marker:text-ink-4">
                {learnedNotes.map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            ) : (
              <p className="text-[14.5px] text-ink-2">
                Nothing yet. neo adds notes as it learns this site.
              </p>
            )}
          </Section>
        </div>
        <div className="flex flex-col gap-5">
          <UltrafastSection skill={skill} runs={runs} />
          <WorksIn skill={skill} />
          <RecentRuns runs={runs} />
        </div>
      </div>

      <Collapsible className="rounded-2xl border border-border-2 bg-card shadow-card">
        <CollapsibleTrigger className="flex w-full items-center gap-2.5 rounded-2xl px-[18px] py-3.5 text-left font-semibold text-[15px] text-ink-2 outline-none hover:text-cyanotype-ink focus-visible:ring-2 focus-visible:ring-ring">
          <FileText aria-hidden className="size-[15px]" />
          <span className="flex-1">View as a file</span>
          <CollapsibleIndicator>
            <ChevronDown aria-hidden className="size-4" />
          </CollapsibleIndicator>
        </CollapsibleTrigger>
        <CollapsibleContent contentClassName="px-[18px] pb-[18px]">
          <CodeBlock code={body} language="text" maxHeight={420} copyable />
        </CollapsibleContent>
      </Collapsible>
    </Page>
  )
}

// ---------------------------------------------------------------------------

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card
      size="compact"
      className="gap-3.5 border-border-2 bg-card px-5 py-[18px] shadow-card"
    >
      <h2 className="font-bold text-[15.5px] text-cyanotype-ink">{title}</h2>
      {children}
    </Card>
  )
}

function WorksIn({ skill }: { skill: Skill }) {
  return (
    <Section title="Works in">
      {skill.linkedAgents.length ? (
        <div className="flex flex-wrap gap-2">
          {skill.linkedAgents.map((agent) => (
            <AgentMark
              key={agent}
              harness={agent}
              className="rounded-md border border-border-2 bg-bg-sunken px-3 py-1.5 font-semibold text-[13.5px] text-ink-2"
            />
          ))}
        </div>
      ) : (
        <p className="text-[14.5px] text-ink-2">
          None of your AI agents have it yet. Connect one and save the task
          again.
        </p>
      )}
    </Section>
  )
}

/** The skill's fast version: its state, how it has been running, and whether it runs with the agent. */
function UltrafastSection({ skill, runs }: { skill: Skill; runs: SkillRun[] }) {
  const ultrafast = skill.ultrafast
  if (!ultrafast || ultrafast.status === 'none') return null
  const counts = runModeCounts(runs)
  const health = healthLine(ultrafast, Date.now())
  const ranFast = counts.ultrafast + counts.handedBack > 0
  const choice = runsChoice(ultrafast, runs)
  return (
    <Section title="Ultrafast">
      <div className="flex flex-col gap-2.5 text-[14.5px] text-ink-2">
        <UltrafastBadge
          ultrafast={ultrafast}
          size="md"
          className="self-start"
        />
        {choice ? null : <p>{ULTRAFAST_STATE[ultrafast.status]}</p>}
        {ultrafast.reason ? (
          <p className="text-[13.5px] text-ink-3">{ultrafast.reason}</p>
        ) : null}
        {health ? <p className="text-[13.5px] text-ink-3">{health}</p> : null}
        {ranFast ? (
          <p className="text-[13.5px] text-ink-3">
            {counts.ultrafast} ultrafast · {counts.handedBack} handed back ·{' '}
            {counts.normal} the normal way
          </p>
        ) : null}
        {choice ? <RunsChoiceBlock skill={skill} choice={choice} /> : null}
        {ultrafast.flowVersion > 0 ? <FlowFile name={skill.name} /> : null}
      </div>
    </Section>
  )
}

const ALWAYS_WITH_REASON: Record<string, string> = {
  pay: 'This skill pays for something.',
  order: 'This skill places an order.',
  send: 'This skill sends something for you.',
  delete: 'This skill deletes something.',
}

const CHIP_CLASS: Record<RunsChoice['chipTone'], string> = {
  accent: 'bg-accent-tint text-accent-ink',
  amber: 'bg-amber-tint text-amber-ink',
  muted: 'bg-bg-sunken text-ink-2',
}

/** "With your agent" or "without your agent" for a skill that changes something. */
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
  const chip = (
    <span
      className={`self-start rounded-full px-2.5 py-1 font-semibold text-[12.5px] ${CHIP_CLASS[choice.chipTone]}`}
    >
      {choice.chip}
    </span>
  )
  if (choice.state === 'read') {
    return (
      <div className="flex flex-col gap-2.5">
        <Note icon={<Eye aria-hidden className="size-4" />}>
          This skill only reads pages, so neo runs it without your agent.
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
          choice was for a different step, so it runs with your agent until you
          choose again.
        </Note>
      ) : null}
      {choice.state === 'always' && detail?.alwaysWithAgent ? (
        <p>{ALWAYS_WITH_REASON[detail.alwaysWithAgent]}</p>
      ) : null}
      {choice.state === 'locked' ? (
        <p>
          ultrafast is off on this computer, so this skill runs with your agent
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
          title="With your agent"
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
            ) : (
              <Zap aria-hidden className="size-4" />
            )
          }
          title="Without your agent"
          time={choice.fastLabel}
          why="neo does it by itself. It only stops if a step can’t find what it expects."
        />
      </RadioGroup>
      {choice.state === 'choose' ? (
        <Note
          tone="amber"
          icon={<MessageCircleQuestion aria-hidden className="size-4" />}
        >
          You haven’t chosen yet, so it runs with your agent. Your agent will
          ask you the next time you use it, or you can choose here.
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
          always run with your agent, so it can check each page first.
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
  title,
  time,
  why,
}: {
  value: 'with_agent' | 'without_agent'
  selected: boolean
  disabled: boolean
  icon: ReactNode
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
      className={`relative flex-row gap-3 px-3.5 py-3 ${disabled ? 'opacity-55' : ''}`}
    >
      <RadioGroupItem
        value={value}
        disabled={disabled}
        aria-labelledby={titleId}
        className={`mt-0.5 ${CARD_TARGET_CLASS}`}
      />
      <span className="flex flex-col gap-0.5">
        <span
          id={titleId}
          className="flex items-center gap-1.5 font-bold text-[14.5px] text-cyanotype-ink"
        >
          {icon}
          {title}
        </span>
        {time ? (
          <span className="font-bold text-[17px] text-cyanotype-ink">
            {time}
          </span>
        ) : null}
        <span className="text-[13.5px] text-ink-2 leading-snug">{why}</span>
      </span>
    </Card>
  )
}

function Note({
  icon,
  tone = 'plain',
  children,
}: {
  icon: ReactNode
  tone?: 'plain' | 'amber'
  children: ReactNode
}) {
  const box =
    tone === 'amber'
      ? 'bg-amber-tint text-ink-2'
      : 'border border-border-2 bg-bg-sunken text-ink-2'
  const iconColor = tone === 'amber' ? 'text-amber-ink' : 'text-ink-3'
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
    <Section title="Recent runs">
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
                  {runModeLabel(run) ? (
                    <span className="flex items-center gap-1 text-[13.5px] text-ink-3">
                      {run.mode === 'ultrafast' ? (
                        <Zap aria-hidden className="size-3 text-accent-ink" />
                      ) : null}
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
        <p className="text-[14.5px] text-ink-2">This skill hasn’t run yet.</p>
      )}
    </Section>
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
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)]">
        <Skeleton className="h-64 rounded-2xl" />
        <Skeleton className="h-64 rounded-2xl" />
      </div>
    </Page>
  )
}
