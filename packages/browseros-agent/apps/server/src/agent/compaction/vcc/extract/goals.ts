/**
 * Goal extraction — vendored from pi-vcc extract/goals.ts.
 * Adapted for BrowserOS NormalizedBlock type.
 */

import { clip, nonEmptyLines } from '../content'
import { collapseSkillLines } from '../skill-collapse'
import type { NormalizedBlock } from '../types'

const SCOPE_CHANGE_RE =
  /\b(instead|actually|change of plan|forget that|new task|switch to|now I want|pivot|let'?s do|stop .* and)\b/i

const TASK_RE =
  /\b(fix|implement|add|create|build|refactor|debug|investigate|update|remove|delete|migrate|deploy|test|write|set up)\b/i

const NOISE_SHORT_RE =
  /^(ok|yes|no|sure|yeah|yep|go|hi|hey|thx|thanks|ok\b.*|y|n|k)\s*[.!?]*$/i

const NON_GOAL_RE =
  /^\s*[[│├└─╭╰]|```|^\s*(=[A-Z]+\(|function |const |let |var |import |export |class )|^(https?:|file:|\/[A-Za-z])|\\n|^\s*For each\b|\bin full\b[^\n]*\b(comments|issue|issues|PRs?|linked)\b/

const TEMPLATE_SIGNAL_RE =
  /^\s*(For each\b|Do NOT implement\b|Analyze and propose\b|If Task\/context\b|Output:\s*$)/i

const truncateAtTemplate = (lines: string[]): string[] => {
  const idx = lines.findIndex((l) => TEMPLATE_SIGNAL_RE.test(l))
  return idx >= 0 ? lines.slice(0, idx) : lines
}

const stripLeadingBullet = (line: string): string =>
  line.replace(/^\s*(?:[-*+]|\d+\.)\s+/, '').trim()

const MAX_GOAL_CHARS = 200

const isSubstantiveGoal = (text: string): boolean => {
  const t = text.trim()
  if (t.length <= 5) return false
  if (t.length > MAX_GOAL_CHARS) return false
  if (NOISE_SHORT_RE.test(t)) return false
  if (NON_GOAL_RE.test(t)) return false
  return true
}

const LEADING_CHARS = 200

export const extractGoals = (blocks: NormalizedBlock[]): string[] => {
  const goals: string[] = []
  let latestScopeChange: string[] | null = null

  for (const b of blocks) {
    if (b.kind !== 'user') continue
    const rawLines = nonEmptyLines(b.text)
    const truncated = truncateAtTemplate(rawLines)
    const lines = collapseSkillLines(truncated.filter(isSubstantiveGoal))
      .map(stripLeadingBullet)
      .filter((l) => l.length > 5)
    if (lines.length === 0) continue

    if (goals.length === 0) {
      goals.push(...lines.slice(0, 6))
      continue
    }

    const leading = b.text.slice(0, LEADING_CHARS)
    if (SCOPE_CHANGE_RE.test(leading)) {
      latestScopeChange = lines.slice(0, 3).map((l) => clip(l, MAX_GOAL_CHARS))
    } else if (TASK_RE.test(leading) && lines[0].length > 15) {
      latestScopeChange = lines.slice(0, 2).map((l) => clip(l, MAX_GOAL_CHARS))
    }
  }

  if (latestScopeChange && latestScopeChange.length > 0) {
    goals.push('[Scope change]', ...latestScopeChange)
  }

  return goals.slice(0, 8)
}
