/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const SKILL_PATH = join('skills', 'browseros', 'SKILL.md')
const SOURCE_SKILL_PATH = fileURLToPath(
  new URL('../../../../resources/skills/browseros/SKILL.md', import.meta.url),
)

export async function loadBrowserOsSkill(
  resourcesDir?: string | null,
): Promise<string> {
  const candidates = [
    ...(resourcesDir?.trim() ? [join(resourcesDir, SKILL_PATH)] : []),
    SOURCE_SKILL_PATH,
  ]
  const failures: string[] = []

  for (const path of new Set(candidates)) {
    try {
      const content = await readFile(path, 'utf8')
      if (isBrowserOsSkill(content)) return content.replace(/\r\n/g, '\n')
      failures.push(`${path}: invalid BrowserOS skill`)
    } catch (error) {
      failures.push(
        `${path}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  throw new Error(`Unable to load BrowserOS skill\n${failures.join('\n')}`)
}

function isBrowserOsSkill(content: string): boolean {
  const normalized = content.replace(/\r\n/g, '\n')
  return (
    normalized.startsWith('---\nname: browseros\n') &&
    normalized.includes('\n---\n')
  )
}
