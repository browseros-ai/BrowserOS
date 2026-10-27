/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill, SkillRun } from '@browseros/claw-api'
import { ChevronDown, FileText, TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { NavLink, useNavigate } from 'react-router'
import { CodeBlock } from '@/components/agents/code-block'
import { SessionStatusBadge } from '@/components/audit/SessionStatusBadge'
import { AgentMark } from '@/components/harness/AgentMark'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Card } from '@/components/motion/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'
import { Phrase } from '@/components/skills/Phrase'
import { RunSkillButton } from '@/components/skills/RunSkillButton'
import { SiteBadge } from '@/components/skills/SiteBadge'
import { SkillActions } from '@/components/skills/SkillActions'
import { Skeleton } from '@/components/ui/skeleton'
import { taskDayAndTime } from '@/screens/audit/audit.helpers'
import { useSkillDetailData } from './skill-detail.data'
import {
  parseSkillBody,
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
            <p className="text-[14px] text-ink-3">
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
