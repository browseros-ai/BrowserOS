import { mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { INLINED_ENV } from '../env'
import { getBuiltinSkillsDir } from '../lib/browseros-dir'
import { logger } from '../lib/logger'
import { DEFAULT_SKILLS } from './defaults'
import { safeBuiltinSkillDir } from './service'
import type { RemoteSkillCatalog, RemoteSkillEntry } from './types'

let syncTimer: ReturnType<typeof setInterval> | null = null

function extractVersion(content: string): string {
  const match = content.match(/^\s*version:\s*["']?([^"'\n]+)["']?/m)
  return match?.[1]?.trim() || '1.0'
}

function extractEnabled(content: string): string | null {
  const match = content.match(/^\s*enabled:\s*["']?(true|false)["']?/m)
  return match?.[1] ?? null
}

function setEnabled(content: string, enabled: string): string {
  return content.replace(
    /^(\s*enabled:\s*)["']?(?:true|false)["']?/m,
    `$1"${enabled}"`,
  )
}

function isValidSkillEntry(entry: unknown): entry is RemoteSkillEntry {
  if (typeof entry !== 'object' || entry === null) return false
  const e = entry as Record<string, unknown>
  return (
    typeof e.id === 'string' &&
    typeof e.version === 'string' &&
    typeof e.content === 'string'
  )
}

function isValidCatalog(data: unknown): data is RemoteSkillCatalog {
  if (typeof data !== 'object' || data === null) return false
  const d = data as Record<string, unknown>
  return (
    typeof d.version === 'number' &&
    Array.isArray(d.skills) &&
    d.skills.every(isValidSkillEntry)
  )
}

export async function fetchRemoteCatalog(): Promise<RemoteSkillCatalog | null> {
  const url = INLINED_ENV.SKILLS_CATALOG_URL || EXTERNAL_URLS.SKILLS_CATALOG
  try {
    const response = await fetch(url, {
      signal: AbortSignal.timeout(TIMEOUTS.SKILLS_FETCH),
    })
    if (!response.ok) {
      logger.warn('Failed to fetch remote skill catalog', {
        status: response.status,
      })
      return null
    }
    const data: unknown = await response.json()
    if (!isValidCatalog(data)) {
      logger.warn('Remote skill catalog has invalid format')
      return null
    }
    return data
  } catch (err) {
    logger.debug('Remote skill catalog unavailable', {
      error: err instanceof Error ? err.message : String(err),
    })
    return null
  }
}

export async function syncBuiltinSkills(): Promise<void> {
  const catalog = await fetchRemoteCatalog()

  const contentMap = new Map<string, { version: string; content: string }>()
  for (const skill of DEFAULT_SKILLS) {
    contentMap.set(skill.id, {
      version: extractVersion(skill.content),
      content: skill.content,
    })
  }
  if (catalog) {
    for (const skill of catalog.skills) {
      contentMap.set(skill.id, {
        version: skill.version,
        content: skill.content,
      })
    }
  }

  for (const [id, source] of contentMap) {
    try {
      await syncOneSkill(id, source)
    } catch (err) {
      logger.warn('Failed to sync builtin skill', {
        id,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  if (catalog) await removeObsoleteSkills(contentMap)
}

async function syncOneSkill(
  id: string,
  source: { version: string; content: string },
): Promise<void> {
  const dir = safeBuiltinSkillDir(id)
  const filePath = join(dir, 'SKILL.md')

  let localContent: string | null = null
  try {
    localContent = await readFile(filePath, 'utf-8')
  } catch {}

  if (localContent && extractVersion(localContent) === source.version) return

  let content = source.content
  if (localContent && extractEnabled(localContent) === 'false') {
    content = setEnabled(content, 'false')
  }

  await mkdir(dir, { recursive: true })
  await writeFile(filePath, content)
}

async function removeObsoleteSkills(
  keepIds: Map<string, unknown>,
): Promise<void> {
  const builtinDir = getBuiltinSkillsDir()
  let entries: string[]
  try {
    entries = await readdir(builtinDir)
  } catch {
    return
  }

  for (const entry of entries) {
    if (entry.startsWith('.') || keepIds.has(entry)) continue
    try {
      const entryPath = join(builtinDir, entry)
      const s = await stat(entryPath)
      if (s.isDirectory()) await rm(entryPath, { recursive: true })
    } catch {}
  }
}

export function startSkillSync(): void {
  if (syncTimer) return
  syncTimer = setInterval(() => {
    syncBuiltinSkills().catch((err) => {
      logger.warn('Skill sync failed', {
        error: err instanceof Error ? err.message : String(err),
      })
    })
  }, TIMEOUTS.SKILLS_SYNC_INTERVAL)
  syncTimer.unref()
}

export function stopSkillSync(): void {
  if (syncTimer) {
    clearInterval(syncTimer)
    syncTimer = null
  }
}
