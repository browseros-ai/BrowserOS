/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure formatters for the Skills list and page. No React, no side effects.
 */

import { verbForTool } from '@/lib/step-sentences'
import type { ReplayVerb } from '@/modules/api/replay.hooks'

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

/** "today", "yesterday", then "on Oct 21" in the reader's locale. */
export function lastUsedLabel(epochMs: number, now: number): string {
  const days = Math.round((startOfDay(now) - startOfDay(epochMs)) / DAY_MS)
  if (days <= 0) return 'today'
  if (days === 1) return 'yesterday'
  const date = new Date(epochMs).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  })
  return `on ${date}`
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

const STUCK_WHILE: Record<ReplayVerb, string> = {
  navigate: 'opening a page',
  read: 'reading a page',
  click: 'clicking on a page',
  type: 'typing on a page',
  attach: 'handling a file',
  submit: 'sending a form',
  done: 'finishing up',
}

/** Where a run that did not finish got stuck, in plain words. */
export function stuckWhile(erroredTool: string | undefined): string {
  return erroredTool
    ? `Got stuck while ${STUCK_WHILE[verbForTool(erroredTool)]}.`
    : 'Got stuck before it finished.'
}
