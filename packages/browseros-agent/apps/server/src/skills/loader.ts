import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import matter from 'gray-matter'
import { getBuiltinSkillsDir, getSkillsDir } from '../lib/browseros-dir'
import { logger } from '../lib/logger'
import { loadSkillsSources, loadSkillsState } from './state'
import type {
  SkillConflict,
  SkillFrontmatter,
  SkillMeta,
  SkillSourceKind,
} from './types'

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

function stateKeyFor(
  skill: Pick<SkillMeta, 'sourceKind' | 'sourceId' | 'id'>,
): string {
  return `${skill.sourceKind}:${skill.sourceId}:${skill.id}`
}

async function parseSkillFile(
  skillMdPath: string,
  dirName: string,
  sourceKind: SkillSourceKind,
  sourceId: string,
  sourceLabel?: string,
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
      builtIn: sourceKind === 'builtin',
      sourceKind,
      sourceId,
      sourceLabel,
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
  sourceKind: SkillSourceKind,
  sourceId: string,
  sourceLabel?: string,
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

    const skill = await parseSkillFile(
      skillMdPath,
      entry,
      sourceKind,
      sourceId,
      sourceLabel,
    )
    if (!skill || seen.has(skill.id)) continue

    seen.add(skill.id)
    skills.push(skill)
  }

  return skills
}

function applyRuntimeState(
  skills: SkillMeta[],
  runtimeState: { skills: Record<string, { enabled: boolean }> },
): SkillMeta[] {
  return skills.map((skill) => {
    const override = runtimeState.skills[stateKeyFor(skill)]
    return override ? { ...skill, enabled: override.enabled } : skill
  })
}

function applyConflicts(skills: SkillMeta[]): SkillMeta[] {
  const grouped = new Map<string, SkillMeta[]>()
  for (const skill of skills) {
    const bucket = grouped.get(skill.id) ?? []
    bucket.push(skill)
    grouped.set(skill.id, bucket)
  }
  return skills.map((skill) => {
    const collisions = grouped.get(skill.id) ?? []
    if (collisions.length <= 1) return skill
    const conflict: SkillConflict = {
      kind: 'duplicate-id',
      collisions: collisions.map((entry) => ({
        sourceKind: entry.sourceKind,
        sourceId: entry.sourceId,
        location: entry.location,
      })),
    }
    return { ...skill, conflict }
  })
}

export async function loadAllSkills(): Promise<SkillMeta[]> {
  const builtinSkills = await scanDir(
    getBuiltinSkillsDir(),
    'builtin',
    'builtin',
  )
  const localSkills = await scanDir(
    getSkillsDir(),
    'local',
    'local',
    'My Skills',
    new Set([PATHS.BUILTIN_DIR_NAME]),
  )

  const sources = await loadSkillsSources()
  const externalSkills = (
    await Promise.all(
      sources.sources
        .filter((source) => source.enabled)
        .map((source) =>
          scanDir(source.path, 'external', source.id, source.label),
        ),
    )
  ).flat()

  const runtimeState = await loadSkillsState()
  return applyConflicts(
    applyRuntimeState(
      [...builtinSkills, ...localSkills, ...externalSkills],
      runtimeState,
    ),
  )
}

export async function loadSkills(): Promise<SkillMeta[]> {
  return (await loadAllSkills()).filter(
    (skill) => skill.enabled && !skill.conflict,
  )
}
