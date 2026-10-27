/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill } from '@browseros/claw-api'
import { Bookmark, Lightbulb, Plus, TriangleAlert } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { NavLink } from 'react-router'
import { CopyButton } from '@/components/copy/CopyButton'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { asHarness } from '@/components/harness/harness.types'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Button } from '@/components/motion/button/base'
import { CARD_TARGET_CLASS, Card } from '@/components/motion/card'
import { Phrase } from '@/components/skills/Phrase'
import { RunSkillButton } from '@/components/skills/RunSkillButton'
import { SiteBadge } from '@/components/skills/SiteBadge'
import { SkillActions } from '@/components/skills/SkillActions'
import { SkillFormDialog } from '@/components/skills/SkillFormDialog'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useSkillsScreenData } from './skills.data'
import { SAVE_PHRASE, usedLabel } from './skills.helpers'

/** Things you ask for often, as cards that each copy the phrase that runs it again. */
export function Skills() {
  const { skills, isLoading, isError } = useSkillsScreenData()
  const [creating, setCreating] = useState(false)
  const now = Date.now()

  return (
    <Page>
      <PageHeader
        title="Skills"
        lede="Things you ask for often. Copy a skill’s phrase and paste it into any of your AI agents to run it again."
        actions={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setCreating(true)}
            className="font-semibold"
          >
            <Plus aria-hidden className="size-4" />
            Make one by hand
          </Button>
        }
      />

      <SaveTip />

      {isError ? (
        <EmptyCard
          tone="error"
          icon={<TriangleAlert className="size-5" />}
          title="Couldn’t load your skills"
          hint="Check that BrowserOS is running, then open this page again."
        />
      ) : isLoading ? (
        <CardsSkeleton />
      ) : skills.length === 0 ? (
        <EmptyCard
          icon={<Bookmark className="size-5" />}
          title="No skills yet"
          hint="Finish a task with one of your AI agents, then ask it to save it as a skill. It shows up here."
        />
      ) : (
        <ul className="grid grid-cols-1 gap-3.5 md:grid-cols-2">
          {skills.map((skill) => (
            <li key={skill.name} className="flex">
              <SkillCard skill={skill} now={now} />
            </li>
          ))}
        </ul>
      )}

      <SkillFormDialog
        mode="create"
        open={creating}
        onOpenChange={setCreating}
      />
    </Page>
  )
}

// ---------------------------------------------------------------------------

/** One quiet line, so the skills stay the first thing on the page. */
function SaveTip() {
  return (
    <p className="flex flex-wrap items-center gap-x-2.5 gap-y-2 text-[14.5px] text-ink-2">
      <span>
        <Lightbulb
          aria-hidden
          className="mr-2 inline size-4 align-[-3px] text-primary"
        />
        To add a skill, finish a task, then tell your AI agent{' '}
        <Phrase>{SAVE_PHRASE}</Phrase>
      </span>
      <CopyButton
        text={SAVE_PHRASE}
        what="the phrase"
        variant="ghost"
        size="sm"
        className="h-[30px] px-2 font-semibold text-[13.5px] text-primary"
      />
    </p>
  )
}

function SkillCard({ skill, now }: { skill: Skill; now: number }) {
  const agents = skill.linkedAgents.flatMap((name) => {
    const harness = asHarness(name)
    return harness ? [harness] : []
  })
  return (
    <Card
      size="compact"
      interactive
      className="relative w-full gap-2.5 border-border-2 bg-card px-4 pt-4 pb-3.5 shadow-card transition-[border-color,box-shadow] hover:border-primary/35 hover:shadow-pop has-[a:focus-visible]:ring-2 has-[a:focus-visible]:ring-ring"
    >
      <div className="flex min-h-8 items-center gap-2.5">
        <SiteBadge site={skill.site} className="size-8 rounded-[10px]" />
        <span className="min-w-0 flex-1 truncate text-[13.5px] text-ink-3">
          {skill.site ?? 'Any website'}
        </span>
        <span className="relative z-10 -my-1.5">
          <SkillActions skill={skill} />
        </span>
      </div>
      <NavLink
        to={`/skills/${encodeURIComponent(skill.name)}`}
        className={cn(
          CARD_TARGET_CLASS,
          'line-clamp-2 font-semibold text-[16px] text-cyanotype-ink leading-[1.4] outline-none',
        )}
      >
        {skill.description}
      </NavLink>
      <p className="text-[13.5px] text-ink-3">
        {usedLabel(skill.runCount, skill.lastRunAt, now)}
      </p>
      <div className="mt-auto flex items-center justify-between gap-3 pt-1.5">
        <RunSkillButton name={skill.name} className="relative z-10" />
        {agents.length > 0 ? (
          <span
            role="img"
            aria-label={`Works in ${agents.join(', ')}`}
            className="flex shrink-0"
          >
            {agents.map((harness, index) => (
              <span
                key={harness}
                className={cn(
                  'grid size-6 place-items-center rounded-full border-2 border-card bg-card shadow-[0_0_0_1px_var(--color-border-2)]',
                  index > 0 && '-ml-1.5',
                )}
              >
                <HarnessIcon harness={harness} className="size-3.5" />
              </span>
            ))}
          </span>
        ) : null}
      </div>
    </Card>
  )
}

function EmptyCard({
  icon,
  title,
  hint,
  tone = 'neutral',
}: {
  icon: ReactNode
  title: string
  hint: string
  tone?: 'neutral' | 'error'
}) {
  return (
    <Card
      size="compact"
      className="flex-row items-center gap-4 border-border-2 bg-card px-[22px] py-5 shadow-card"
    >
      <span
        aria-hidden
        className={cn(
          'grid size-11 shrink-0 place-items-center rounded-full',
          tone === 'error'
            ? 'bg-red-tint text-red-ink'
            : 'bg-accent-tint text-primary',
        )}
      >
        {icon}
      </span>
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="font-bold text-[16px] text-cyanotype-ink">
          {title}
        </span>
        <span className="text-[14.5px] text-ink-2">{hint}</span>
      </span>
    </Card>
  )
}

function CardsSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-3.5 md:grid-cols-2">
      {['s1', 's2', 's3', 's4'].map((id) => (
        <Card
          key={id}
          size="compact"
          className="gap-3 border-border-2 bg-card px-4 pt-4 pb-3.5 shadow-card"
        >
          <div className="flex items-center gap-2.5">
            <Skeleton className="size-8 rounded-[10px]" />
            <Skeleton className="h-3.5 w-1/3" />
          </div>
          <Skeleton className="h-4 w-4/5" />
          <Skeleton className="h-3.5 w-2/5" />
          <Skeleton className="h-[34px] w-32 rounded-full" />
        </Card>
      ))}
    </div>
  )
}
