import type { SkillMeta } from './types'

const SKILL_BEHAVIORAL_INSTRUCTION = `The following skills provide specialized instructions for specific tasks.
When a task matches a skill's description, use filesystem_read to load the SKILL.md at the listed location before proceeding.
When a skill references relative paths (e.g., scripts/), resolve them against the skill's directory (the parent of SKILL.md) and use absolute paths in tool calls.`

const SKILL_CHAT_MODE_INSTRUCTION = `The following skills describe specialized workflows. In read-only chat mode you cannot read SKILL.md from disk (no filesystem tools). Use each skill's name and description as optional high-level guidance when relevant — do not claim you followed detailed steps from a file you did not load.`

function escapeXml(str: string): string {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

export interface BuildSkillsCatalogOptions {
  chatMode?: boolean
}

export function buildSkillsCatalog(
  skills: SkillMeta[],
  options?: BuildSkillsCatalogOptions,
): string {
  if (skills.length === 0) return ''

  const chatMode = options?.chatMode ?? false
  const instruction = chatMode
    ? SKILL_CHAT_MODE_INSTRUCTION
    : SKILL_BEHAVIORAL_INSTRUCTION

  const skillEntries = skills
    .map((s) => {
      if (chatMode) {
        return `<skill>
<name>${escapeXml(s.name)}</name>
<description>${escapeXml(s.description)}</description>
</skill>`
      }
      return `<skill>
<name>${escapeXml(s.name)}</name>
<description>${escapeXml(s.description)}</description>
<location>${escapeXml(s.location)}</location>
</skill>`
    })
    .join('\n')

  return `${instruction}

<available_skills>
${skillEntries}
</available_skills>`
}
