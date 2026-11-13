/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure formatters for the Skills list and page. No React, no side effects.
 */

import type { Skill, SkillRun, SkillUltrafast } from '@browseros/claw-api'
import { HARNESSES, type Harness } from '@/components/harness/harness.types'
import type { AnimatedBadgeStatus } from '@/components/motion/animated-badge'
import { verbForTool } from '@/lib/step-sentences'
import type { ReplayVerb } from '@/modules/api/replay.hooks'
import { siteHost } from '@/modules/favicon/favicon.helpers'

/** Every skill is namespaced under this prefix so it never collides with a
 *  user's own agent skills and the whole set autocompletes on `/neo`. */
export const NEO_SKILL_PREFIX = 'neo-'

/**
 * The canonical, neo-namespaced name for a user-typed suffix. Mirrors the
 * server: a bare suffix is prefixed, and an over-typed `neo-` is not doubled.
 */
export function neoName(suffix: string): string {
  return `${NEO_SKILL_PREFIX}${suffix.replace(/^neo-/, '')}`
}

/**
 * The phrase a person pastes into an AI agent to run a skill. Plain words
 * rather than a prefix, since each agent has its own (`/name` in Claude Code,
 * `$name` in Codex) and every one understands being asked by name.
 */
export function skillCommand(name: string): string {
  return `use the ${name} skill`
}

/**
 * Split a rendered SKILL.md into its editable structured fields so the edit
 * form can pre-fill them. Reads the numbered `## Steps` and the bulleted
 * `## Learned from past runs` sections. The auto-generated first step that
 * records the run is dropped, since the server re-adds it on save.
 */
export function parseSkillBody(body: string): {
  steps: string[]
  learnedNotes: string[]
} {
  const steps: string[] = []
  const learnedNotes: string[] = []
  let section: 'steps' | 'learned' | null = null
  for (const line of body.split('\n')) {
    const trimmed = line.trim()
    if (trimmed.startsWith('## ')) {
      const title = trimmed.slice(3).toLowerCase()
      section = title.startsWith('steps')
        ? 'steps'
        : title.startsWith('learned')
          ? 'learned'
          : null
      continue
    }
    if (section === 'steps') {
      const match = line.match(/^\s*\d+\.\s+(.*)$/)
      const step = match?.[1]?.trim()
      if (step && !step.includes('mark_skill_run')) {
        steps.push(step)
      }
    } else if (section === 'learned') {
      const match = line.match(/^\s*-\s+(.*)$/)
      const note = match?.[1]?.trim()
      if (note) {
        learnedNotes.push(note)
      }
    }
  }
  return { steps, learnedNotes }
}

/** What a person tells their AI agent, after a task, to save it here. */
export const SAVE_PHRASE = 'save this as a skill in neo'

const DAY_MS = 24 * 60 * 60 * 1000

function startOfDay(ms: number): number {
  const day = new Date(ms)
  day.setHours(0, 0, 0, 0)
  return day.getTime()
}

/** "today", "yesterday", then "Oct 21" in the reader's locale. */
export function lastUsedLabel(epochMs: number, now: number): string {
  const days = Math.round((startOfDay(now) - startOfDay(epochMs)) / DAY_MS)
  if (days <= 0) return 'today'
  if (days === 1) return 'yesterday'
  return new Date(epochMs).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  })
}

/** "Used 4 times · last used yesterday", or "Not used yet". */
export function usedLabel(
  runCount: number,
  lastRunAt: number | undefined,
  now: number,
): string {
  if (runCount <= 0) return 'Not used yet'
  const times = runCount === 1 ? 'Used once' : `Used ${runCount} times`
  return lastRunAt === undefined
    ? times
    : `${times} · last used ${lastUsedLabel(lastRunAt, now)}`
}

/**
 * The sites a skill works on, by host ("trmnl.com" for "https://trmnl.com").
 * A skill stores one free-text site, which a person or an agent may have
 * written as a full address or as a comma list ("x.com, reddit.com").
 */
export function skillSites(site: string | undefined): string[] {
  return (site ?? '')
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => siteHost(entry)?.replace(/^www\./, '') ?? entry)
}

/**
 * The connected agents any skill is linked into, once, in the usual harness
 * order. A skill records its links when it is saved and keeps them after an
 * agent is disconnected, so the saved links alone can name agents that are gone.
 */
export function worksInAgents(
  skills: Pick<Skill, 'linkedAgents'>[],
  connected: readonly Harness[],
): Harness[] {
  return HARNESSES.filter(
    (harness) =>
      connected.includes(harness) &&
      skills.some((skill) => skill.linkedAgents.includes(harness)),
  )
}

const STUCK_WHILE: Record<ReplayVerb, string> = {
  navigate: 'opening a page',
  read: 'reading a page',
  click: 'clicking on a page',
  type: 'typing on a page',
  attach: 'handling a file',
  submit: 'sending a form',
  browse: 'clicking through a page',
  done: 'finishing up',
}

/** Where a run that did not finish got stuck, in plain words. */
export function stuckWhile(erroredTool: string | undefined): string {
  return erroredTool
    ? `Got stuck while ${STUCK_WHILE[verbForTool(erroredTool)]}.`
    : 'Got stuck before it finished.'
}

export interface UltrafastBadgeMeta {
  label: string
  tone: AnimatedBadgeStatus
}

/** How a skill's ultrafast state reads on a badge, or nothing for a plain skill. */
export function ultrafastBadge(
  ultrafast: SkillUltrafast | undefined,
): UltrafastBadgeMeta | null {
  switch (ultrafast?.status) {
    case 'verified':
      return { label: 'Ultrafast', tone: 'success' }
    case 'degraded':
      return { label: 'Ultrafast · needs a repair', tone: 'warning' }
    case 'broken':
      return { label: 'Ultrafast paused', tone: 'danger' }
    case 'pending':
      return { label: 'Becoming ultrafast', tone: 'info' }
    case 'off':
      return { label: 'Runs the normal way', tone: 'neutral' }
    default:
      return null
  }
}

/** "about 1.2 s a run" from a median in milliseconds. */
export function runSpeed(p50Ms: number): string {
  const seconds = Math.max(0.1, Math.round(p50Ms / 100) / 10)
  return `about ${seconds} s a run`
}

/** "Checked today · repaired yesterday · about 1.2 s a run". */
export function healthLine(ultrafast: SkillUltrafast, now: number): string {
  const parts: string[] = []
  if (ultrafast.verifiedAt !== undefined) {
    parts.push(`Checked ${lastUsedLabel(ultrafast.verifiedAt, now)}`)
  }
  if (ultrafast.repairedAt !== undefined) {
    parts.push(`repaired ${lastUsedLabel(ultrafast.repairedAt, now)}`)
  }
  if (ultrafast.p50Ms !== undefined) parts.push(runSpeed(ultrafast.p50Ms))
  return parts.join(' · ')
}

const HANDOVER_CAUSE: Record<string, string> = {
  site_changed: 'the site changed',
  input_only: 'this input did not match',
  transient: 'the page was slow or asked for a check',
}

/** How a run was carried out, or nothing for a run done the normal way. */
export function runModeLabel(
  run: Pick<SkillRun, 'mode' | 'handoverStep' | 'handoverCause'>,
): string | null {
  if (run.mode === 'ultrafast') return 'Ultrafast'
  if (run.mode !== 'handed_back') return null
  const step = run.handoverStep ? ` at step ${run.handoverStep}` : ''
  const cause = run.handoverCause
    ? HANDOVER_CAUSE[run.handoverCause]
    : undefined
  return `Took over${step}${cause ? `: ${cause}` : ''}`
}

/** How many runs went each way, for the skill page. */
export function runModeCounts(runs: Pick<SkillRun, 'mode'>[]): {
  ultrafast: number
  handedBack: number
  normal: number
} {
  return runs.reduce(
    (counts, run) => {
      if (run.mode === 'ultrafast') counts.ultrafast += 1
      else if (run.mode === 'handed_back') counts.handedBack += 1
      else counts.normal += 1
      return counts
    },
    { ultrafast: 0, handedBack: 0, normal: 0 },
  )
}

/** Which of the skill page's "with or without your agent" states a skill is in. */
export type RunsChoiceState =
  | 'read'
  | 'choose'
  | 'without'
  | 'with'
  | 'reask'
  | 'always'
  | 'locked'

export interface RunsChoice {
  state: RunsChoiceState
  /** The option shown as selected. */
  selected: 'with_agent' | 'without_agent'
  /** "Without your agent" can't be picked: always-with skills and a plan that is off. */
  withoutLocked: boolean
  /** Neither option can be picked: the plan is off. */
  disabled: boolean
  chip: string
  chipTone: 'accent' | 'amber' | 'muted'
  /** How long it takes without the agent, from the verified median. */
  fastLabel: string | null
  /** How long it takes with the agent, from runs the agent did itself. */
  slowLabel: string | null
}

/** "about 11 sec" or "about 1 min", the way the skill page talks about time. */
export function durationLabel(ms: number | undefined | null): string | null {
  if (!ms || ms <= 0) return null
  const seconds = Math.max(1, Math.round(ms / 1000))
  if (seconds < 60) return `about ${seconds} sec`
  return `about ${Math.round(seconds / 60)} min`
}

function median(values: number[]): number | null {
  if (!values.length) return null
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.floor(sorted.length / 2)] ?? null
}

/**
 * Where a skill stands on running with or without the agent, or null when the skill page has
 * nothing to say (no fast version yet).
 */
export function runsChoice(
  ultrafast: SkillUltrafast | undefined,
  runs: Pick<SkillRun, 'mode' | 'durationMs'>[],
): RunsChoice | null {
  if (!ultrafast) return null
  const live =
    ultrafast.status === 'verified' || ultrafast.status === 'degraded'
  if (!live) return null
  const fastLabel = durationLabel(ultrafast.p50Ms)
  const slowLabel = durationLabel(
    median(
      runs
        .filter((run) => run.mode !== 'ultrafast' && run.durationMs)
        .map((run) => run.durationMs as number),
    ),
  )
  const base = { fastLabel, slowLabel, withoutLocked: false, disabled: false }
  if (ultrafast.locked) {
    return {
      ...base,
      state: 'locked',
      selected: 'with_agent',
      withoutLocked: true,
      disabled: true,
      chip: 'Off',
      chipTone: 'muted',
    }
  }
  const choice = ultrafast.runs
  if (!choice) {
    return {
      ...base,
      state: 'read',
      selected: 'without_agent',
      chip: 'Ready',
      chipTone: 'accent',
    }
  }
  if (choice.alwaysWithAgent) {
    return {
      ...base,
      state: 'always',
      selected: 'with_agent',
      withoutLocked: true,
      chip: 'Always with your agent',
      chipTone: 'muted',
    }
  }
  if (choice.chosen && choice.mode === 'without_agent') {
    return {
      ...base,
      state: 'without',
      selected: 'without_agent',
      chip: 'Runs without your agent',
      chipTone: 'accent',
    }
  }
  if (choice.chosen) {
    return {
      ...base,
      state: 'with',
      selected: 'with_agent',
      chip: 'Runs with your agent',
      chipTone: 'muted',
    }
  }
  if (choice.reask) {
    return {
      ...base,
      state: 'reask',
      selected: 'with_agent',
      chip: 'Asks again',
      chipTone: 'amber',
    }
  }
  return {
    ...base,
    state: 'choose',
    selected: 'with_agent',
    chip: 'Choose how it runs',
    chipTone: 'amber',
  }
}

/** Who chose and how, in the words the skill page uses. */
export function chosenLine(
  choice: NonNullable<SkillUltrafast['runs']>,
  now: number,
): string | null {
  if (!choice.chosen || !choice.chosenAt) return null
  const when = new Date(choice.chosenAt).toLocaleDateString(undefined, {
    day: 'numeric',
    month: 'short',
  })
  const today =
    new Date(now).toDateString() === new Date(choice.chosenAt).toDateString()
  const day = today ? 'today' : `on ${when}`
  if (choice.via === 'chat') {
    const who = choice.client ? ` with ${choice.client}` : ''
    return `You chose this in chat${who} ${day}`
  }
  return `You chose this in BrowserOS ${day}`
}
