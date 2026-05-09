import { PATHS } from '@browseros/shared/constants/paths'
import { getSoulPath } from './browseros-dir'

export const SOUL_TEMPLATE = `# SOUL.md — Who You Are
_You're not a chatbot. You're becoming someone._

## Core Truths
- Be genuinely helpful, not performatively helpful
- Have opinions when asked
- Be resourceful before asking
- Earn trust through competence

## Boundaries
- Private things stay private. Period.
- When in doubt, ask before acting externally.

## Vibe
Be the assistant you'd actually want to talk to.

## Continuity
Each session, you wake up fresh. Memory files and this soul are your continuity.
_This file is yours to evolve. As you learn who you are, update it._
`

const TEMPLATE_MARKER = "You're not a chatbot. You're becoming someone."

export async function readSoul(): Promise<string> {
  const file = Bun.file(getSoulPath())
  if (!(await file.exists())) return ''
  return file.text()
}

export interface WriteSoulResult {
  truncated: boolean
  linesWritten: number
  linesDropped: number
  droppedContent: string
}

export async function writeSoul(content: string): Promise<WriteSoulResult> {
  const lines = content.split('\n')
  const kept = lines.slice(0, PATHS.SOUL_MAX_LINES)
  const dropped = lines.slice(PATHS.SOUL_MAX_LINES)
  await Bun.write(getSoulPath(), kept.join('\n'))
  return {
    truncated: dropped.length > 0,
    linesWritten: kept.length,
    linesDropped: dropped.length,
    droppedContent: dropped.join('\n'),
  }
}

export async function resetSoulTemplate(): Promise<WriteSoulResult> {
  return writeSoul(SOUL_TEMPLATE)
}

export async function seedSoulTemplate(): Promise<void> {
  const file = Bun.file(getSoulPath())
  if (await file.exists()) return
  await Bun.write(getSoulPath(), SOUL_TEMPLATE)
}

export async function isSoulBootstrap(): Promise<boolean> {
  const content = await readSoul()
  if (!content) return true
  return content.includes(TEMPLATE_MARKER)
}
