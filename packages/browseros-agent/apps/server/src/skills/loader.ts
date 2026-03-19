import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import matter from 'gray-matter'
import { getBuiltinSkillsDir, getSkillsDir } from '../lib/browseros-dir'
import { logger } from '../lib/logger'
import type { SkillFrontmatter, SkillMeta } from './types'

async function isDirectory(dirPath: string): Promise<boolean> {
  try {
    const s = await stat(dirPath)
    return s.isDirectory()
  } catch {
    return false
  }
}

export function isValidFrontmatter(data: unknown): data is SkillFrontmatter {
  if (typeof data !== 'object' || data === null) return false
  const d = data as Record<string, unknown>
  return (
    typeof d.name === 'string' &&
    d.name.length > 0 &&
    typeof d.description === 'string' &&
    d.description.length > 0
  )
}

async function parseSkillFile(
  skillMdPath: string,
  dirName: string,
  builtIn: boolean,
): Promise<SkillMeta | null> {
  try {
    const content = await readFile(skillMdPath, 'utf-8')
    const { data } = matter(content)

    if (!isValidFrontmatter(data)) {
      logger.warn('Skill missing required frontmatter fields', {
        path: skillMdPath,
        dirName,
      })
      return null
    }

    const meta = data.metadata
    return {
      id: dirName,
      name: meta?.['display-name'] || data.name,
      description: data.description,
      location: skillMdPath,
      enabled: meta?.enabled !== 'false',
      version: meta?.version,
      builtIn,
    }
  } catch (err) {
    logger.warn('Failed to parse skill', {
      path: skillMdPath,
      error: err instanceof Error ? err.message : String(err),
    })
    return null
  }
}

async function scanDir(
  dir: string,
  builtIn: boolean,
  skipDirs?: Set<string>,
): Promise<SkillMeta[]> {
  let entries: string[]
  try {
    entries = await readdir(dir)
  } catch {
    return []
  }

  const skills: SkillMeta[] = []
  const seen = new Set<string>()

  for (const entry of entries) {
    if (skipDirs?.has(entry)) continue
    const entryPath = join(dir, entry)
    if (!(await isDirectory(entryPath))) continue

    const skillMdPath = join(entryPath, 'SKILL.md')
    try {
      await stat(skillMdPath)
    } catch {
      continue
    }

    const skill = await parseSkillFile(skillMdPath, entry, builtIn)
    if (!skill || seen.has(skill.id)) continue

    seen.add(skill.id)
    skills.push(skill)
  }

  return skills
}

export async function loadAllSkills(): Promise<SkillMeta[]> {
  const builtinSkills = await scanDir(getBuiltinSkillsDir(), true)
  const userSkills = await scanDir(
    getSkillsDir(),
    false,
    new Set([PATHS.BUILTIN_DIR_NAME]),
  )
  return [...builtinSkills, ...userSkills]
}

export async function loadSkills(): Promise<SkillMeta[]> {
  const all = await loadAllSkills()
  return all.filter((s) => s.enabled)
}
