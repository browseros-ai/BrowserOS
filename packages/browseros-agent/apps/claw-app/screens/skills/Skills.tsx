/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill } from '@browseros/claw-api'
import { Bookmark, Globe, Plus, TriangleAlert } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { NavLink } from 'react-router'
import { CopyButton } from '@/components/copy/CopyButton'
import { Favicon } from '@/components/favicon/Favicon'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
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
import {
  linkedAgentsOf,
  SAVE_PHRASE,
  skillSites,
  usedLabel,
} from './skills.helpers'

/** Two columns at the page's full width, one on narrow screens. */
const GRID_CLASS =
  'grid grid-cols-[repeat(auto-fill,minmax(min(360px,100%),1fr))] gap-3'

/** Things you ask for often, as cards that each copy the phrase that runs it again. */
export function Skills() {
  const { skills, isLoading, isError } = useSkillsScreenData()
  const [creating, setCreating] = useState(false)
  const agents = linkedAgentsOf(skills)
  const now = Date.now()

  return (
    <Page className="gap-7 md:gap-7">
      <PageHeader
        title="Skills"
        lede="Things you ask for often. Copy one into any of your AI agents to run it again."
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

      <div className="flex flex-col gap-5">
        {agents.length > 0 ? <WorksIn agents={agents} /> : null}

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
          <>
            <EmptyCard
              icon={<Bookmark className="size-5" />}
              title="No skills yet"
              hint="Finish a task with one of your AI agents, then ask it to save it as a skill. It shows up here."
            />
            <div className={GRID_CLASS}>
              <SaveTipCard />
            </div>
          </>
        ) : (
          <ul className={GRID_CLASS}>
            {skills.map((skill) => (
              <li key={skill.name} className="flex">
                <SkillCard skill={skill} now={now} />
              </li>
            ))}
            <li className="flex">
              <SaveTipCard />
            </li>
          </ul>
        )}
      </div>

      <SkillFormDialog
        mode="create"
        open={creating}
        onOpenChange={setCreating}
      />
    </Page>
  )
}

// ---------------------------------------------------------------------------

/**
 * The connected agents, shown once for every skill. A skill records the
 * agents it was linked into when it was saved, so they can differ between
 * skills and the line makes no claim about how many.
 */
function WorksIn({ agents }: { agents: Harness[] }) {
  return (
    <div className="flex items-center gap-2.5">
      <span role="img" aria-label={agents.join(', ')} className="flex shrink-0">
        {agents.map((harness, index) => (
          <span
            key={harness}
            className={cn(
              'grid size-6 place-items-center rounded-full bg-card shadow-[0_0_0_1px_var(--color-border-2)]',
              index > 0 && '-ml-2',
            )}
          >
            <HarnessIcon harness={harness} className="size-3.5" />
          </span>
        ))}
      </span>
      <p className="text-[14px] text-ink-2 leading-[18px]">
        Works in your connected agents.
      </p>
    </div>
  )
}

/**
 * The title links to the skill's page across the whole card. Copy and the
 * menu sit above that link only because they are positioned and come after
 * it in the DOM, with no z-index: SkillActions renders its edit dialog in
 * place, so a stacking context around it would trap the dialog under the
 * cards after it. That is why the card is a grid: the menu is drawn on the
 * top row but written last. The card is not `interactive` either, since its
 * press scale is a transform, which would re-anchor that fixed dialog to the
 * card while pressed.
 */
function SkillCard({ skill, now }: { skill: Skill; now: number }) {
  const sites = skillSites(skill.site)
  const favicon = sites.length === 1 ? sites[0] : undefined
  return (
    <Card
      size="compact"
      className="grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2.5 border-border-2 bg-card px-4 pt-4 pb-3.5 shadow-card hover:border-primary/35 has-[a:focus-visible]:ring-2 has-[a:focus-visible]:ring-ring"
    >
      <span
        aria-hidden
        className="col-start-1 row-start-1 grid size-7 place-items-center rounded-md bg-accent-tint text-ink-3"
      >
        {favicon ? (
          <Favicon site={favicon} className="size-4 rounded-[3px]" />
        ) : (
          <Globe className="size-4" />
        )}
      </span>
      <span className="col-start-2 row-start-1 truncate text-[13px] text-ink-3 leading-4">
        {sites.length > 0 ? sites.join(', ') : 'Any website'}
      </span>
      <NavLink
        to={`/skills/${encodeURIComponent(skill.name)}`}
        title={skill.description}
        className={cn(
          CARD_TARGET_CLASS,
          'col-span-full row-start-2 truncate font-semibold text-[15.5px] text-cyanotype-ink leading-[21px] outline-none',
        )}
      >
        {skill.description}
      </NavLink>
      <div className="col-span-full row-start-3 flex items-center justify-between gap-3">
        <span className="truncate text-[13px] text-ink-3 leading-4">
          {usedLabel(skill.runCount, skill.lastRunAt, now)}
        </span>
        <RunSkillButton
          name={skill.name}
          variant="tinted"
          className="relative h-[30px] shrink-0 px-2.5 text-[13px] [&_svg]:size-[13px]"
        />
      </div>
      <SkillActions
        skill={skill}
        menuClassName="relative col-start-3 row-start-1 -my-2 -mr-2.5 justify-self-end"
      />
    </Card>
  )
}

/**
 * The last cell of the grid: how a skill gets here, with the phrase to copy.
 * Copy sits on the title row because the hint and phrase already fill a
 * two-column card's line.
 */
function SaveTipCard() {
  return (
    <div className="flex min-h-[108px] w-full flex-col justify-center gap-2 rounded-xl border border-accent-ink/25 border-dashed px-[18px] py-4">
      <div className="flex items-center justify-between gap-3">
        <span className="flex items-center gap-2 font-semibold text-[15px] text-cyanotype-ink leading-5">
          <Plus aria-hidden className="size-4 text-accent-ink" />
          Add a skill
        </span>
        <CopyButton
          text={SAVE_PHRASE}
          what="the phrase"
          variant="ghost"
          size="sm"
          className="-my-[5px] -mr-2 h-[30px] px-2 font-semibold text-[13.5px] text-primary"
        />
      </div>
      <p className="text-[13.5px] text-ink-2 leading-5">
        Finish a task, then tell your agent{' '}
        <Phrase className="whitespace-nowrap">{SAVE_PHRASE}</Phrase>
      </p>
    </div>
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
    <div className={GRID_CLASS}>
      {['s1', 's2', 's3', 's4'].map((id) => (
        <Card
          key={id}
          size="compact"
          className="gap-2.5 border-border-2 bg-card px-4 pt-4 pb-3.5 shadow-card"
        >
          <div className="flex items-center gap-2.5">
            <Skeleton className="size-7" />
            <Skeleton className="h-3.5 w-1/3" />
          </div>
          <Skeleton className="my-[3px] h-4 w-4/5" />
          <div className="flex items-center justify-between">
            <Skeleton className="h-3.5 w-2/5" />
            <Skeleton className="h-[30px] w-[72px]" />
          </div>
        </Card>
      ))}
    </div>
  )
}
