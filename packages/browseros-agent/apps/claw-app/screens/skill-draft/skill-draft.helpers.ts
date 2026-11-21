/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure rules for making a skill from a finished task: who sees the button, which state it shows,
 * what the sheet says when a draft could not be written, and what a reviewed draft saves as.
 */

import type {
  SessionSummary,
  SkillCreate,
  SkillDraft,
  SkillDraftFailure,
  SkillDraftInput,
  SkillDraftStatus,
} from '@browseros/claw-api'
import { neoName } from '@/screens/skills/skills.helpers'

export const ULTRAFAST_FEATURE = 'claw.ultrafast'

/** Whether this plan can make skills from tasks: every ultrafast plan can. */
export function canDraftSkills(
  features: readonly string[] | undefined,
): boolean {
  return features?.includes(ULTRAFAST_FEATURE) === true
}

/** A finished, or stopped but error-free, task with at least two steps and no skill yet can become one. */
export function isDraftable(task: SessionSummary): boolean {
  const ended =
    task.status === 'done' ||
    (task.status === 'cancelled' && task.errorCount === 0)
  return ended && task.dispatchCount >= 2 && !task.skillName
}

/** The newest task in a list that could become a skill: its button is always shown. */
export function newestDraftableId(
  tasks: readonly SessionSummary[],
): string | undefined {
  return tasks.find(isDraftable)?.sessionId
}

/** What the "Make it a skill" button shows, shared by every view of the task. */
export type MakeSkillState =
  | 'idle'
  | 'writing'
  | 'review'
  | 'draft'
  | 'saving'
  | 'saved'
  | 'error'
  | 'limited'

export function makeSkillState({
  status,
  sheetOpen,
  drafting,
  saving,
  now,
}: {
  status: SkillDraftStatus | undefined
  sheetOpen: boolean
  drafting: boolean
  saving: boolean
  now: number
}): MakeSkillState {
  if (saving) return 'saving'
  if (status?.status === 'saved') return 'saved'
  if (drafting || status?.status === 'writing') return 'writing'
  if (status?.status === 'ready') return sheetOpen ? 'review' : 'draft'
  if (status?.status === 'failed') {
    return isLimited(status.failure, now) ? 'limited' : 'error'
  }
  return 'idle'
}

/** A click does nothing while a draft is being written, read or saved. */
export function isBusy(state: MakeSkillState): boolean {
  return state === 'writing' || state === 'review' || state === 'saving'
}

function isLimited(
  failure: SkillDraftFailure | undefined,
  now: number,
): boolean {
  if (failure?.code !== 'limit_reached') return false
  const resets = failure.resetsAt ? Date.parse(failure.resetsAt) : Number.NaN
  return Number.isNaN(resets) || resets > now
}

/** "1 December", the day a monthly limit resets, in UTC as the worker counts it. */
export function resetDay(resetsAt: string | undefined): string | undefined {
  if (!resetsAt) return undefined
  const date = new Date(resetsAt)
  if (Number.isNaN(date.getTime())) return undefined
  return new Intl.DateTimeFormat('en-GB', {
    day: 'numeric',
    month: 'long',
    timeZone: 'UTC',
  }).format(date)
}

/** What the sheet says when a draft could not be written. */
export interface DraftProblem {
  title: string
  detail: string
  /** The line above the "save this as a skill in neo" phrase. */
  agentLine: string
  /** Whether "Try again" is offered (not for limits, which a retry cannot pass). */
  retry: boolean
}

const AGENT_CAN_SAVE =
  'Your agent can still save one for you. After a task, tell it:'
const IF_IT_KEEPS =
  'If it keeps happening, your agent can save it for you. After a task, tell it:'

export function draftProblem(
  failure: SkillDraftFailure | undefined,
): DraftProblem {
  switch (failure?.code) {
    case 'limit_reached':
      return failure.scope === 'month'
        ? {
            title: 'You’ve reached this month’s skill limit.',
            detail: resetDay(failure.resetsAt)
              ? `It resets on ${resetDay(failure.resetsAt)}.`
              : 'It resets at the start of next month.',
            agentLine: AGENT_CAN_SAVE,
            retry: false,
          }
        : {
            title: 'You’ve made 100 skills today.',
            detail: 'You can make more tomorrow.',
            agentLine: AGENT_CAN_SAVE,
            retry: false,
          }
    case 'drafts_paused':
      return {
        title: 'Making skills is paused for now.',
        detail: 'Try again later.',
        agentLine: AGENT_CAN_SAVE,
        retry: false,
      }
    case 'draft_unusable':
      return {
        title: 'This task was hard to turn into a skill.',
        detail: 'Try another one, or ask your agent to save it.',
        agentLine: AGENT_CAN_SAVE,
        retry: true,
      }
    default:
      return {
        title: 'We couldn’t write this skill right now.',
        detail: 'Try again in a minute.',
        agentLine: IF_IT_KEEPS,
        retry: true,
      }
  }
}

/** The person's edits to a draft in the review sheet. */
export interface DraftEdits {
  title: string
  description: string
  steps: string[]
  inputs: SkillDraftInput[]
}

export function editsFrom(draft: SkillDraft): DraftEdits {
  return {
    title: draft.title,
    description: draft.description,
    steps: [...draft.steps],
    inputs: [...draft.inputs],
  }
}

/** Whether the edited draft can be saved: a name and at least one step. */
export function canSave(edits: DraftEdits): boolean {
  return (
    edits.title.trim().length > 0 &&
    edits.steps.some((step) => step.trim().length > 0)
  )
}

/**
 * The skill a reviewed draft saves as. The card shows the title, so it is the description; the
 * one-line "what it does" goes above the steps in SKILL.md.
 */
/** Where "Make it a skill" was shown, as analytics names it. */
export type SkillCtaSurface = 'home' | 'history' | 'task_page'

/** What the person changed before saving, as booleans and counts: never the words themselves. */
export function savedDraftProps(
  draft: SkillDraft,
  edits: DraftEdits,
): Record<string, boolean | number> {
  const steps = edits.steps.map((step) => step.trim()).filter(Boolean)
  return {
    edited_name: edits.title.trim() !== draft.title.trim(),
    edited_steps: steps.join('\n') !== draft.steps.join('\n'),
    edited_inputs:
      JSON.stringify(edits.inputs) !== JSON.stringify(draft.inputs),
    steps: steps.length,
    inputs: edits.inputs.length,
  }
}

export function toSkillCreate(
  draft: SkillDraft,
  edits: DraftEdits,
  sessionId: string,
): SkillCreate {
  const steps = edits.steps.map((step) => step.trim()).filter(Boolean)
  const used = steps.join('\n')
  return {
    name: neoName(draft.name),
    description: edits.title.trim(),
    site: draft.site,
    steps,
    learnedNotes: draft.notes,
    sourceSessionId: sessionId,
    summary: edits.description.trim() || undefined,
    inputs: edits.inputs.filter((input) => used.includes(`{${input.name}}`)),
  }
}

/** "Flipkart" from "flipkart.com", for "From your Flipkart task". */
export function siteName(site: string | undefined): string | undefined {
  const host = site?.replace(/^www\./, '').split('/')[0]
  const label = host?.split('.')[0]
  return label ? label.charAt(0).toUpperCase() + label.slice(1) : undefined
}
