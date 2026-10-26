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
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import {
  Alert,
  AlertContent,
  AlertDescription,
} from '@/components/motion/alert'
import { Button } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import { Phrase } from '@/components/skills/Phrase'
import { RunSkillButton } from '@/components/skills/RunSkillButton'
import { SiteBadge } from '@/components/skills/SiteBadge'
import { SkillActions } from '@/components/skills/SkillActions'
import { SkillFormDialog } from '@/components/skills/SkillFormDialog'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useSkillsScreenData } from './skills.data'
import { SAVE_PHRASE, skillCommand, usedLabel } from './skills.helpers'

/** Things you ask for often, each with a short phrase that runs it again. */
export function Skills() {
  const { skills, isLoading, isError } = useSkillsScreenData()
  const [creating, setCreating] = useState(false)
  const now = Date.now()

  return (
    <Page>
      <PageHeader
        title="Saved tasks"
        lede="Things you ask for often. Each one has a short phrase that runs it again in any of your AI agents."
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
          title="Couldn’t load your saved tasks"
          hint="Check that BrowserOS is running, then open this page again."
        />
      ) : isLoading ? (
        <RowsSkeleton />
      ) : skills.length === 0 ? (
        <EmptyCard
          icon={<Bookmark className="size-5" />}
          title="No saved tasks yet"
          hint="Your first one shows up here after your AI agents save it."
        />
      ) : (
        <Card
          size="compact"
          className="gap-0 overflow-hidden border-border-2 bg-card py-0 shadow-card"
        >
          <ul className="flex flex-col divide-y divide-border-2">
            {skills.map((skill) => (
              <SavedTaskRow key={skill.name} skill={skill} now={now} />
            ))}
          </ul>
        </Card>
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

function SaveTip() {
  return (
    <Alert
      variant="info"
      className="rounded-2xl px-4 py-3"
      contentClassName="flex-wrap items-center gap-3"
    >
      <Lightbulb aria-hidden className="size-5 shrink-0 text-primary" />
      <AlertContent className="min-w-0 flex-1 space-y-0">
        <AlertDescription className="text-[14.5px] text-ink">
          To save a task, let it finish, then tell your AI agents{' '}
          <Phrase>{SAVE_PHRASE}</Phrase>
        </AlertDescription>
      </AlertContent>
      <CopyButton
        text={SAVE_PHRASE}
        what="the phrase"
        size="sm"
        className="h-[34px] shrink-0 font-semibold text-[13.5px]"
      />
    </Alert>
  )
}

function SavedTaskRow({ skill, now }: { skill: Skill; now: number }) {
  return (
    <li className="flex flex-wrap items-center gap-x-4 gap-y-3 px-4 py-3.5 sm:flex-nowrap sm:px-[18px]">
      <SiteBadge />
      <NavLink
        to={`/skills/${encodeURIComponent(skill.name)}`}
        className="flex min-w-0 flex-1 basis-[calc(100%-56px)] flex-col gap-1 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring sm:basis-auto"
      >
        <span className="font-semibold text-[15.5px] text-cyanotype-ink hover:underline">
          {skill.description}
        </span>
        <span className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[13.5px] text-ink-3">
          <Phrase>{skillCommand(skill.name)}</Phrase>
          <span>{usedLabel(skill.runCount, skill.lastRunAt, now)}</span>
        </span>
      </NavLink>
      <span className="ml-14 flex shrink-0 items-center gap-1.5 sm:ml-0">
        <RunSkillButton name={skill.name} />
        <SkillActions skill={skill} />
      </span>
    </li>
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
    <Card
      size="compact"
      className="gap-0 overflow-hidden border-border-2 bg-card py-0 shadow-card"
    >
      <div className="flex flex-col divide-y divide-border-2">
        {['s1', 's2', 's3'].map((id) => (
          <div key={id} className="flex items-center gap-4 px-[18px] py-3.5">
            <Skeleton className="size-10 rounded-[10px]" />
            <div className="flex flex-1 flex-col gap-2">
              <Skeleton className="h-4 w-1/2" />
              <Skeleton className="h-3.5 w-1/3" />
            </div>
            <Skeleton className="h-[34px] w-32 rounded-full" />
          </div>
        ))}
      </div>
    </Card>
  )
}
