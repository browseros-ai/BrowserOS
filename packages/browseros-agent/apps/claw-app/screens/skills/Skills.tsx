/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill } from '@browseros/claw-api'
import { Bookmark, Globe, Lightbulb, Plus, TriangleAlert } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { NavLink } from 'react-router'
import { CopyButton } from '@/components/copy/CopyButton'
import { Favicon } from '@/components/favicon/Favicon'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Button } from '@/components/motion/button/base'
import { CARD_TARGET_CLASS, Card } from '@/components/motion/card'
import { Phrase } from '@/components/skills/Phrase'
import { RunSkillButton } from '@/components/skills/RunSkillButton'
import { SkillActions } from '@/components/skills/SkillActions'
import { SkillFormDialog } from '@/components/skills/SkillFormDialog'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useSkillsScreenData } from './skills.data'
import { runsLabel, SAVE_PHRASE } from './skills.helpers'

/** Things you ask for often, one row each, led by the name an agent runs it by. */
export function Skills() {
  const { skills, isLoading, isError } = useSkillsScreenData()
  const [creating, setCreating] = useState(false)

  return (
    <Page className="gap-6 md:gap-6">
      <PageHeader
        title="Skills"
        lede="Things you ask for often. Ask any of your AI agents to use one by name."
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
        <RowsSkeleton />
      ) : skills.length === 0 ? (
        <EmptyCard
          icon={<Bookmark className="size-5" />}
          title="No skills yet"
          hint="Finish a task with one of your AI agents, then ask it to save it as a skill. It shows up here."
        />
      ) : (
        <ul className="flex flex-col border-border-2 border-t">
          {skills.map((skill) => (
            <SkillRow key={skill.name} skill={skill} />
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

/**
 * The name links to the skill's page across the whole row. Copy and the menu
 * sit above that link only because they are positioned and come later in the
 * row; a z-index there would trap the edit dialog, which renders in place,
 * under the rows below it. The menu stays hidden until the row is hovered or
 * focused, except on touch screens, which have no hover to reveal it. On
 * phones, Copy steps aside while the menu is open so the name keeps its room.
 */
function SkillRow({ skill }: { skill: Skill }) {
  return (
    <li className="group relative flex items-center gap-3.5 border-border-2 border-b px-1 py-[13px] transition-colors hover:bg-card has-[a:focus-visible]:ring-2 has-[a:focus-visible]:ring-ring has-[a:focus-visible]:ring-inset">
      <span aria-hidden className="flex w-5 shrink-0 justify-center text-ink-3">
        {skill.site ? (
          <Favicon site={skill.site} className="size-4 rounded-[3px]" />
        ) : (
          <Globe className="size-4" />
        )}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-[3px]">
        <NavLink
          to={`/skills/${encodeURIComponent(skill.name)}`}
          className={cn(
            CARD_TARGET_CLASS,
            'truncate font-medium font-mono text-[14px] text-cyanotype-ink leading-[18px] outline-none',
          )}
        >
          {skill.name}
        </NavLink>
        <span className="truncate text-[13.5px] text-ink-3 leading-[18px]">
          {skill.description}
        </span>
        <RunCount
          runCount={skill.runCount}
          className="truncate text-left leading-[18px] sm:hidden"
        />
      </span>
      <RunCount
        runCount={skill.runCount}
        className="hidden w-24 shrink-0 text-right leading-4 sm:block"
      />
      <span className="relative flex w-[76px] shrink-0 justify-end max-sm:group-has-[[aria-expanded=true]]:hidden">
        <RunSkillButton
          name={skill.name}
          variant="ghost"
          className="h-[30px] px-2.5 text-accent-ink [&_svg]:size-[13px]"
        />
      </span>
      <SkillActions
        skill={skill}
        menuClassName="relative opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100 has-[[aria-expanded=true]]:opacity-100 pointer-coarse:opacity-100"
      />
    </li>
  )
}

/** Its own column on wider screens; under the description on phones. */
function RunCount({
  runCount,
  className,
}: {
  runCount: number
  className?: string
}) {
  return (
    <span
      className={cn(
        'text-[13px]',
        runCount > 0 ? 'text-ink-2' : 'text-ink-4',
        className,
      )}
    >
      {runsLabel(runCount)}
    </span>
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

function RowsSkeleton() {
  return (
    <div className="flex flex-col border-border-2 border-t">
      {['s1', 's2', 's3', 's4'].map((id) => (
        <div
          key={id}
          className="flex items-center gap-3.5 border-border-2 border-b px-1 py-[13px]"
        >
          <span className="flex w-5 shrink-0 justify-center">
            <Skeleton className="size-4 rounded-[3px]" />
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-[3px] py-0.5">
            <Skeleton className="h-3.5 w-48" />
            <Skeleton className="h-3.5 w-3/5" />
          </div>
          <Skeleton className="h-[30px] w-[76px]" />
        </div>
      ))}
    </div>
  )
}
