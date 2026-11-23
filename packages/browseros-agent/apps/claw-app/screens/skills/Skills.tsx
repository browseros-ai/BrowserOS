/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill } from '@browseros/claw-api'
import {
  ChevronRight,
  Globe,
  Lightbulb,
  MessageCircleQuestion,
  Plus,
  RefreshCw,
  Search,
  TriangleAlert,
} from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { NavLink } from 'react-router'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { CopyButton } from '@/components/copy/CopyButton'
import { useCopyText } from '@/components/copy/copy.hooks'
import { Favicon } from '@/components/favicon/Favicon'
import type { Harness } from '@/components/harness/harness.types'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Button } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import { useUpgradeToasts } from '@/components/skills/NowUltrafast'
import { AgentFaces, PasteHint } from '@/components/skills/PasteHint'
import { SkillActions } from '@/components/skills/SkillActions'
import { SkillFormDialog } from '@/components/skills/SkillFormDialog'
import {
  CopiedLine,
  CopyCueContent,
  SkillPhrase,
} from '@/components/skills/SkillPhrase'
import { UltrafastNudge } from '@/components/skills/UltrafastNudge'
import { UltrafastState } from '@/components/skills/UltrafastState'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useSkillsScreenData } from './skills.data'
import {
  awaitsFirstRun,
  cardStats,
  matchesQuery,
  SAVE_PHRASE,
  skillAgents,
  skillCommand,
  skillSites,
  ultrafastMark,
} from './skills.helpers'

/** As many columns as fit at about 330px each; one on narrow screens. */
const GRID_CLASS =
  'grid grid-cols-[repeat(auto-fill,minmax(min(330px,100%),1fr))] gap-4'

/** A list longer than this gets a search box; it stays while a search is active. */
const SEARCH_FROM = 6

/** Things you ask for often, as cards: a click copies the phrase that runs one again. */
export function Skills() {
  const { skills, connected, nudge, isLoading, isError } = useSkillsScreenData()
  const [creating, setCreating] = useState(false)
  const [query, setQuery] = useState('')
  useUpgradeToasts(skills)
  const empty = !isLoading && !isError && skills.length === 0
  const shown = skills.filter((skill) => matchesQuery(skill, query))

  return (
    <Page className="gap-6 md:gap-6">
      <PageHeader
        title="Skills"
        lede={
          empty
            ? undefined
            : 'Click a skill to copy it, then paste it into your AI agent.'
        }
        actions={
          empty ? undefined : (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setCreating(true)}
              className="font-semibold"
            >
              <Plus aria-hidden className="size-4" />
              Make one by hand
            </Button>
          )
        }
      />

      {nudge ? (
        <UltrafastNudge surface="skills_list" className="-mt-2">
          Your skills run at normal speed. With <UltrafastWordmark />, neo runs
          them in seconds.
        </UltrafastNudge>
      ) : null}

      {isError ? (
        <ErrorCard />
      ) : isLoading ? (
        <CardsSkeleton />
      ) : empty ? (
        <EmptySkills agents={connected} onMake={() => setCreating(true)} />
      ) : (
        <div className="flex flex-col gap-5">
          {skills.length > SEARCH_FROM || query.trim() ? (
            <SkillSearch
              query={query}
              onQuery={setQuery}
              shown={shown.length}
              total={skills.length}
            />
          ) : null}
          {shown.length ? (
            <ul className={GRID_CLASS}>
              {shown.map((skill) => (
                <li key={skill.name} className="flex">
                  <SkillCard
                    skill={skill}
                    agents={skillAgents(skill, connected)}
                  />
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-[14.5px] text-ink-2">
              No skills match “{query.trim()}”.
            </p>
          )}
          <SaveTip />
        </div>
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

function SkillSearch({
  query,
  onQuery,
  shown,
  total,
}: {
  query: string
  onQuery: (query: string) => void
  shown: number
  total: number
}) {
  return (
    <div className="flex items-center gap-3">
      <label className="relative max-w-[380px] flex-1">
        <span className="sr-only">Find a skill</span>
        <Search
          aria-hidden
          className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-ink-3"
        />
        <input
          type="search"
          defaultValue={query}
          onInput={(event) => onQuery(event.currentTarget.value)}
          placeholder="Find a skill or a site"
          className="h-10 w-full rounded-[11px] border border-border bg-card pr-3 pl-9 text-[14.5px] text-cyanotype-ink shadow-card outline-none placeholder:text-ink-3 focus-visible:border-primary focus-visible:ring-3 focus-visible:ring-primary/20"
        />
      </label>
      <span aria-live="polite" className="text-[13.5px] text-ink-3">
        {query.trim() ? `${shown} of ${total} skills` : `${total} skills`}
      </span>
    </div>
  )
}

/** One quiet line under the list, so the skills stay the first thing on the page. */
function SaveTip() {
  return (
    <p className="flex flex-wrap items-center gap-x-2.5 gap-y-2 text-[14px] text-ink-3">
      <Lightbulb aria-hidden className="size-4" />
      <span>Want another skill? Finish a task, then tell your agent</span>
      <code className="rounded-[7px] bg-bg-sunken px-2 py-0.5 font-mono text-[12.5px] text-ink-2">
        {SAVE_PHRASE}
      </code>
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
 * The whole card copies the skill's phrase, through a button stretched over it. The menu and
 * Details sit above that button only because they are positioned and come after it in the DOM,
 * with no z-index: SkillActions renders its edit dialog in place, so a stacking context (or a
 * transform, such as a hover lift) around it would trap or re-anchor that dialog. After a copy the
 * footer swaps its stats for the paste hint in the same fixed slot, so the grid never moves.
 */
function SkillCard({ skill, agents }: { skill: Skill; agents: Harness[] }) {
  const { copied, shown, copy } = useCopyText()
  // A hand-edited SKILL.md can carry a blank description; the name keeps the card titled.
  const title = skill.description.trim() || skill.name
  const sites = skillSites(skill.site)
  const favicon = sites.length === 1 ? sites[0] : undefined
  return (
    <Card
      size="compact"
      className="group/card relative flex w-full flex-col gap-3 rounded-[18px] border-border-2 bg-card px-4 pt-4 pb-3 shadow-card transition-[box-shadow,border-color] duration-300 hover:border-accent-tint-2 hover:shadow-pop has-[[data-copy]:focus-visible]:ring-2 has-[[data-copy]:focus-visible]:ring-ring motion-reduce:transition-none"
    >
      <button
        type="button"
        data-copy
        aria-label={`Copy skill: ${title}`}
        onClick={async () => {
          if (await copy(skillCommand(skill.name))) {
            track(AnalyticsEvent.SkillCopied, { surface: 'card' })
          }
        }}
        className="absolute inset-0 cursor-pointer rounded-[inherit] outline-none"
      />
      <div className="flex min-w-0 items-center gap-2.5">
        <span
          aria-hidden
          className="grid size-7 shrink-0 place-items-center rounded-md bg-accent-tint text-ink-3"
        >
          {favicon ? (
            <Favicon site={favicon} className="size-4 rounded-[3px]" />
          ) : (
            <Globe className="size-4" />
          )}
        </span>
        <span className="truncate text-[13.5px] text-ink-3">
          {sites.length > 0 ? sites.join(', ') : 'Any website'}
        </span>
        <SkillActions
          skill={skill}
          withOpen
          menuClassName="relative -my-2 -mr-2.5 ml-auto"
        />
      </div>
      <h3
        title={title}
        className="line-clamp-2 font-bold text-[16px] text-cyanotype-ink leading-[1.38] tracking-[-0.01em]"
      >
        {title}
      </h3>
      <div
        className={cn(
          'mt-auto flex min-h-[52px] items-center gap-3 rounded-xl border bg-accent-tint py-2 pr-2 pl-4 transition-[border-color,box-shadow] duration-300',
          copied
            ? 'border-primary/55 ring-4 ring-primary/12'
            : 'border-accent-tint-2',
        )}
      >
        <SkillPhrase
          name={skill.name}
          className="flex-1 text-[13.5px] leading-[1.5]"
        />
        <span
          aria-hidden
          className={cn(
            'inline-flex h-9 shrink-0 items-center gap-[7px] rounded-[10px] border px-3.5 font-semibold text-[13.5px] shadow-card transition-colors duration-200',
            copied
              ? 'animate-copy-pop border-primary bg-primary text-primary-foreground'
              : 'border-accent-tint-2 bg-card text-accent-ink group-hover/card:border-primary group-hover/card:bg-primary group-hover/card:text-primary-foreground',
          )}
        >
          <CopyCueContent copied={copied} />
        </span>
      </div>
      <div className="pointer-events-none relative h-[30px]">
        <div
          className={cn(
            'absolute inset-0 flex items-center justify-between gap-2 transition-[opacity,translate] duration-350 ease-[cubic-bezier(0.16,1,0.3,1)]',
            copied && '-translate-y-2 opacity-0',
          )}
        >
          <span className="flex min-w-0 items-center gap-2 text-[13px] text-ink-3">
            <CardStatsLine skill={skill} />
          </span>
          <NavLink
            to={`/skills/${encodeURIComponent(skill.name)}`}
            tabIndex={copied ? -1 : undefined}
            className={cn(
              'relative inline-flex shrink-0 items-center gap-0.5 rounded-lg py-1 pr-1.5 pl-2 font-semibold text-[13px] text-ink-2 outline-none transition-colors hover:bg-bg-sunken hover:text-cyanotype-ink focus-visible:ring-2 focus-visible:ring-ring',
              !copied && 'pointer-events-auto',
            )}
          >
            Details
            <ChevronRight aria-hidden className="size-3.5" />
          </NavLink>
        </div>
        <div
          aria-live="polite"
          className={cn(
            'absolute inset-0 flex items-center transition-[opacity,translate] duration-350 ease-[cubic-bezier(0.16,1,0.3,1)]',
            !copied && 'translate-y-2 opacity-0',
          )}
        >
          {shown ? <PasteHint agents={agents} className="w-full" /> : null}
        </div>
      </div>
    </Card>
  )
}

/** A choice the skill still needs, or its ultrafast state, speed and runs. */
function CardStatsLine({ skill }: { skill: Skill }) {
  const stats = cardStats(skill)
  if (stats.choice) {
    return (
      <span className="flex items-center gap-1.5 font-semibold text-amber-ink">
        {stats.choice === 'Asks again' ? (
          <RefreshCw aria-hidden className="size-3.5" />
        ) : (
          <MessageCircleQuestion aria-hidden className="size-3.5" />
        )}
        {stats.choice}
      </span>
    )
  }
  const marked = ultrafastMark(skill.ultrafast, awaitsFirstRun(skill)) !== null
  return (
    <>
      <UltrafastState
        ultrafast={skill.ultrafast}
        afterFirstRun={awaitsFirstRun(skill)}
        wordmarkClassName="text-[13.5px]"
      >
        {stats.speed ? <span>{stats.speed}</span> : null}
      </UltrafastState>
      {marked && stats.uses ? (
        <span aria-hidden className="h-3 w-px shrink-0 bg-border-strong" />
      ) : null}
      {stats.uses ? (
        <span className="truncate whitespace-nowrap">{stats.uses}</span>
      ) : null}
    </>
  )
}

/** No skills yet: what a skill is, and the two steps to the first one. */
function EmptySkills({
  agents,
  onMake,
}: {
  agents: Harness[]
  onMake: () => void
}) {
  const { copied, copy } = useCopyText()
  return (
    <section className="mx-auto flex w-full max-w-[620px] flex-col items-center pt-2 text-center">
      <span
        aria-hidden
        className="grid size-[104px] place-items-center rounded-[30px] border border-accent-tint-2 bg-accent-tint shadow-card"
      >
        <img
          src="/icons/browserclaw.svg"
          alt=""
          className="size-[60px] rounded-[15px]"
        />
      </span>
      <h2 className="mt-6 font-extrabold text-[28px] text-cyanotype-ink leading-[1.2] tracking-[-0.025em]">
        No skills yet
      </h2>
      <p className="mt-2.5 max-w-[46ch] text-[16px] text-ink-2">
        A skill is a task neo has learned, so any of your AI agents can run it
        again from one line.
      </p>
      <div className="mt-6 flex w-full flex-col gap-3.5 rounded-[20px] border border-border-2 bg-card-tint p-5 text-left shadow-card">
        <EmptyStep number={1}>Finish a task with your AI agent</EmptyStep>
        <EmptyStep number={2}>Then tell it</EmptyStep>
        <button
          type="button"
          aria-label={`Copy ${SAVE_PHRASE}`}
          onClick={() => void copy(SAVE_PHRASE)}
          className={cn(
            'group/phrase -mt-1.5 flex min-h-[52px] w-full cursor-pointer items-center gap-3 rounded-xl border bg-card py-2 pr-2 pl-4 text-left outline-none transition-[border-color,box-shadow] duration-300 focus-visible:ring-2 focus-visible:ring-ring',
            copied
              ? 'border-primary/55 ring-4 ring-primary/12'
              : 'border-accent-tint-2',
          )}
        >
          <SkillPhrase
            name="neo"
            before="save this as a skill in"
            after=""
            className="flex-1 text-[13.5px]"
          />
          <span
            aria-hidden
            className={cn(
              'inline-flex h-9 shrink-0 items-center gap-[7px] rounded-[10px] border px-3.5 font-semibold text-[13.5px] shadow-card transition-colors duration-200',
              copied
                ? 'animate-copy-pop border-primary bg-primary text-primary-foreground'
                : 'border-accent-tint-2 bg-card text-accent-ink group-hover/phrase:border-primary group-hover/phrase:bg-primary group-hover/phrase:text-primary-foreground',
            )}
          >
            <CopyCueContent copied={copied} />
          </span>
        </button>
        <div className="flex flex-wrap items-center gap-3 text-[14.5px]">
          <CopiedLine
            copied={copied}
            before="Works in any of your connected agents"
            after="Copied. Paste it after your next task"
          />
          <AgentFaces agents={agents} limit={7} />
        </div>
      </div>
      <p className="mt-4 flex items-center gap-2 text-[14px] text-ink-3">
        Or
        <button
          type="button"
          onClick={onMake}
          className="inline-flex items-center gap-1 rounded-sm font-semibold text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Plus aria-hidden className="size-4" />
          Make one by hand
        </button>
      </p>
    </section>
  )
}

function EmptyStep({
  number,
  children,
}: {
  number: number
  children: ReactNode
}) {
  return (
    <p className="flex items-center gap-2.5 font-semibold text-[14px] text-cyanotype-ink">
      <span className="grid size-[22px] shrink-0 place-items-center rounded-[7px] bg-accent-tint font-bold text-[12px] text-accent-ink">
        {number}
      </span>
      {children}
    </p>
  )
}

function ErrorCard() {
  return (
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
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="font-bold text-[16px] text-cyanotype-ink">
          Couldn’t load your skills
        </span>
        <span className="text-[14.5px] text-ink-2">
          Check that BrowserOS is running, then open this page again.
        </span>
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
          className="gap-3 rounded-[18px] border-border-2 bg-card px-4 pt-4 pb-3 shadow-card"
        >
          <div className="flex items-center gap-2.5">
            <Skeleton className="size-7" />
            <Skeleton className="h-3.5 w-1/3" />
          </div>
          <Skeleton className="my-[3px] h-4 w-4/5" />
          <Skeleton className="h-[52px] rounded-xl" />
          <Skeleton className="h-3.5 w-2/5" />
        </Card>
      ))}
    </div>
  )
}
