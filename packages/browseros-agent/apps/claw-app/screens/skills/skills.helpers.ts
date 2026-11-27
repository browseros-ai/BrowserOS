/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure formatters for the Skills list and page. No React, no side effects.
 */

import type { Skill, SkillRun, SkillUltrafast } from '@browseros/claw-api'
import type { Harness } from '@/components/harness/harness.types'
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
 * The connected agents a skill can be pasted into: the ones it is linked into. A skill saved with
 * no links (an empty list) can go to any connected agent, so it shows them all.
 */
export function skillAgents(
  skill: Pick<Skill, 'linkedAgents'>,
  connected: readonly Harness[],
): Harness[] {
  if (!skill.linkedAgents.length) return [...connected]
  return connected.filter((harness) => skill.linkedAgents.includes(harness))
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

export interface UltrafastMark {
  /** The wordmark dims when the fast version isn't running as it should. */
  dim: boolean
  /** The state after the wordmark, in words; none for a verified skill. */
  word: string | null
  tone: 'amber' | 'red' | 'muted'
}

/**
 * How a skill's ultrafast state reads beside the wordmark, or nothing for a plain skill.
 * `afterFirstRun` marks a skill made from a task that has not run yet: it has no flow, but its
 * first run in an agent makes it ultrafast, so it says so. A plan without ultrafast (`locked`)
 * shows no mark at all, since every skill then runs the normal way.
 */
export function ultrafastMark(
  ultrafast: SkillUltrafast | undefined,
  afterFirstRun = false,
): UltrafastMark | null {
  if (ultrafast?.locked === true) return null
  switch (ultrafast?.status) {
    case 'verified':
      return { dim: false, word: null, tone: 'muted' }
    case 'degraded':
      return { dim: true, word: 'needs a repair', tone: 'amber' }
    case 'broken':
      return { dim: true, word: 'paused', tone: 'red' }
    case 'pending':
      return { dim: true, word: 'on the way', tone: 'muted' }
    case 'off':
      return { dim: true, word: 'off', tone: 'muted' }
    default:
      return afterFirstRun
        ? { dim: true, word: 'after its first run', tone: 'muted' }
        : null
  }
}

/** A skill made from a finished task that is waiting for the run that makes it ultrafast. */
export function awaitsFirstRun(skill: Skill): boolean {
  return (
    skill.origin === 'history' &&
    skill.runCount === 0 &&
    skill.ultrafast?.status === 'none' &&
    skill.ultrafast.locked !== true
  )
}

export interface LearningStep {
  text: string
  state: 'done' | 'now' | 'todo'
}

/** How long the skill page keeps checking a pending fast version after it opens. */
export const VERIFYING_WINDOW_MS = 2 * 60_000

/** How often the skill page checks a pending fast version inside that window. */
export const VERIFYING_POLL_MS = 3000

/**
 * The skill page's next check of a pending fast version, or false to stop. It runs one check past
 * the window so the page re-renders and stops saying neo is checking.
 */
export function verifyingPoll(
  skill: Skill | undefined,
  openedAt: number,
  now: number,
): number | false {
  if (skill?.ultrafast?.status !== 'pending') return false
  return now - openedAt < VERIFYING_WINDOW_MS + VERIFYING_POLL_MS
    ? VERIFYING_POLL_MS
    : false
}

/** Whether a pending fast version still reads as being checked: only inside the window. */
export function stillVerifying(openedAt: number, updatedAt: number): boolean {
  return updatedAt - openedAt < VERIFYING_WINDOW_MS
}

/**
 * What the skill page says while a task skill's first run turns into its fast version: the agent
 * handed neo a flow and neo is checking it, or, once the page stops checking, that it finishes on
 * the next run. Null once it is verified, or for any other skill.
 */
export function firstRunLearning(
  skill: Skill,
  checking = true,
): LearningStep[] | null {
  const ultrafast = skill.ultrafast
  if (
    skill.origin !== 'history' ||
    ultrafast?.status !== 'pending' ||
    ultrafast.verifiedAt ||
    ultrafast.locked === true
  ) {
    return null
  }
  const done: LearningStep[] = [
    { text: 'Your agent is doing the task the normal way', state: 'done' },
    { text: 'neo is learning the steps', state: 'done' },
  ]
  if (!checking)
    return [...done, { text: 'Finishes on its next run', state: 'todo' }]
  return [
    ...done,
    { text: 'neo is checking the fast version', state: 'now' },
    { text: 'Next time it takes seconds', state: 'todo' },
  ]
}

/** A skill made from a task whose fast version is verified: the one-time "now ultrafast" moment. */
export function turnedUltrafast(skill: Skill): boolean {
  return (
    skill.origin === 'history' &&
    skill.ultrafast?.status === 'verified' &&
    skill.ultrafast.verifiedAt !== undefined
  )
}

/** Whether a skill's next run goes without the agent: read flows, or writes chosen to. */
export function runsByItself(skill: Skill): boolean {
  const state = runsChoice(skill.ultrafast, [])?.state
  return state === 'read' || state === 'without'
}

/** The big figure on the "now ultrafast" strip: whole seconds a run, at least 1. */
export function upgradeSeconds(p50Ms: number): number {
  return Math.max(1, Math.round(p50Ms / 1000))
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
  /** `ultrafast` chips read after the wordmark. */
  chipTone: 'ultrafast' | 'amber' | 'muted'
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

/** How long a run done the normal way usually takes, "about 41 sec", from the skill's runs. */
export function normalRunLabel(
  runs: Pick<SkillRun, 'mode' | 'durationMs'>[],
): string | null {
  return durationLabel(
    median(
      runs
        .filter((run) => run.mode !== 'ultrafast' && run.durationMs)
        .map((run) => run.durationMs as number),
    ),
  )
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
  const slowLabel = normalRunLabel(runs)
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
      chip: 'ready',
      chipTone: 'ultrafast',
    }
  }
  if (choice.alwaysWithAgent) {
    return {
      ...base,
      state: 'always',
      selected: 'with_agent',
      withoutLocked: true,
      chip: 'Always step by step',
      chipTone: 'muted',
    }
  }
  if (choice.chosen && choice.mode === 'without_agent') {
    return {
      ...base,
      state: 'without',
      selected: 'without_agent',
      chip: 'runs in one go',
      chipTone: 'ultrafast',
    }
  }
  if (choice.chosen) {
    return {
      ...base,
      state: 'with',
      selected: 'with_agent',
      chip: 'Runs step by step',
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

/** What a skill card says under its phrase: a choice to make, or its speed and runs. */
export interface CardStats {
  /** A write skill that still needs "with or without your agent" says so instead of stats. */
  choice: 'Choose how it runs' | 'Asks again' | null
  /** "1.8 s", only when its runs really go the fast way. */
  speed: string | null
  /** "32 runs", "1 run", or "Not used yet" for a skill with no ultrafast word to show. */
  uses: string | null
}

/** "1.8 s" from a median in milliseconds, the short form for a card. */
function shortSpeed(p50Ms: number): string {
  return `${Math.max(0.1, Math.round(p50Ms / 100) / 10)} s`
}

export function cardStats(skill: Skill): CardStats {
  const state = runsChoice(skill.ultrafast, [])?.state
  if (state === 'choose') {
    return { choice: 'Choose how it runs', speed: null, uses: null }
  }
  if (state === 'reask')
    return { choice: 'Asks again', speed: null, uses: null }
  const p50Ms = skill.ultrafast?.p50Ms
  const fast =
    skill.ultrafast?.status === 'verified' && runsByItself(skill) && p50Ms
  const marked = ultrafastMark(skill.ultrafast, awaitsFirstRun(skill)) !== null
  return {
    choice: null,
    speed: fast ? shortSpeed(p50Ms) : null,
    uses:
      skill.runCount > 0
        ? skill.runCount === 1
          ? '1 run'
          : `${skill.runCount} runs`
        : marked
          ? null
          : 'Not used yet',
  }
}

/** Whether a skill matches what someone typed in the search: its title, name or site. */
export function matchesQuery(
  skill: Pick<Skill, 'name' | 'description' | 'site'>,
  query: string,
): boolean {
  const words = query.trim().toLowerCase()
  if (!words) return true
  return [skill.description, skill.name, skill.site ?? ''].some((field) =>
    field.toLowerCase().includes(words),
  )
}

/** A skill's fast flow as analytics reports it: its status, or `locked` once the plan ended. */
export function ultrafastStateToken(
  ultrafast: SkillUltrafast | undefined,
): string {
  if (!ultrafast) return 'none'
  return ultrafast.locked ? 'locked' : ultrafast.status
}

/** How much a skill is used: the skill page's value row changes shape with it. */
export type UsageLevel = 'never' | 'rare' | 'sometimes' | 'lot'

export function usageLevel(runCount: number): UsageLevel {
  if (runCount <= 0) return 'never'
  if (runCount < 3) return 'rare'
  if (runCount < 10) return 'sometimes'
  return 'lot'
}

/** One figure on the skill page's value row. */
export interface ValueTile {
  value: string
  unit?: string
  label: string
  /** Speed tiles read in the ultrafast blue. */
  fast: boolean
}

/** What the skill page says above its stats: tiles once it has run, an invite before. */
export type SkillValue =
  | { kind: 'invite'; title: string; body: string; fast: boolean }
  | { kind: 'tiles'; tiles: ValueTile[] }

/**
 * What a skill has done for this person, by how much it is used. Speed shows only when its runs
 * really go the fast way (verified, and read-only or chosen to run without the agent), since a
 * skill that runs with the agent takes the agent's own time.
 */
export function skillValue(
  skill: Skill,
  runs: Pick<SkillRun, 'mode' | 'durationMs'>[],
  now: number,
): SkillValue {
  const p50Ms = skill.ultrafast?.p50Ms
  const fast =
    skill.ultrafast?.status === 'verified' && runsByItself(skill) && p50Ms
      ? p50Ms
      : null
  const normal = normalRunLabel(runs)
  const level = usageLevel(skill.runCount)
  if (level === 'never') {
    if (fast) {
      return {
        kind: 'invite',
        fast: true,
        title: 'Ready when you are',
        body: normal
          ? `It takes ${durationLabel(fast)} a run, down from ${normal} the normal way.`
          : `It takes ${durationLabel(fast)} a run.`,
      }
    }
    if (awaitsFirstRun(skill)) {
      return {
        kind: 'invite',
        fast: false,
        title: 'Run it once and neo learns it',
        body: 'Your agent does it the normal way the first time. Next time it takes seconds.',
      }
    }
    if (skill.ultrafast?.status === 'pending' && !skill.ultrafast.locked) {
      return {
        kind: 'invite',
        fast: false,
        title: 'Almost ready',
        body: 'Your agent is turning this skill into a fast version.',
      }
    }
    return {
      kind: 'invite',
      fast: false,
      title: 'Try it the first time',
      body: 'Copy it, paste it into your agent, and it does the task for you.',
    }
  }
  const lastUsed =
    skill.lastRunAt === undefined ? null : lastUsedLabel(skill.lastRunAt, now)
  const tiles: ValueTile[] = []
  if (fast) {
    tiles.push({
      value: `${Math.max(0.1, Math.round(fast / 100) / 10)}`,
      unit: 's',
      label: normal ? `a run, down from ${normal}` : 'a run',
      fast: true,
    })
  }
  const runsUnit = skill.runCount === 1 ? 'run' : 'runs'
  if (fast && level !== 'rare' && lastUsed) {
    tiles.push({
      value: String(skill.runCount),
      unit: runsUnit,
      label: 'so far',
      fast: false,
    })
    tiles.push({ value: lastUsed, label: 'last used', fast: false })
  } else {
    tiles.push({
      value: String(skill.runCount),
      unit: runsUnit,
      label: lastUsed ? `last used ${lastUsed}` : 'so far',
      fast: false,
    })
  }
  return { kind: 'tiles', tiles }
}
