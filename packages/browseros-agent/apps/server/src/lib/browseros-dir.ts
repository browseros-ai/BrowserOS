import { unlinkSync } from 'node:fs'
import { readdir, rm, stat, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import type { ServerDiscoveryConfig } from '@browseros/shared/types/server-config'
import { ensureDirectory } from './ensure-directory'
import { logger } from './logger'

export function getBrowserosDir(): string {
  const override = process.env.BROWSEROS_DIR?.trim()
  if (override) {
    return override
  }
  const dirName =
    process.env.NODE_ENV === 'development'
      ? PATHS.DEV_BROWSEROS_DIR_NAME
      : PATHS.BROWSEROS_DIR_NAME
  return join(homedir(), dirName)
}

export function logDevelopmentBrowserosDir(): void {
  if (process.env.NODE_ENV !== 'development') return
  logger.info(`Using development BrowserOS directory: ${getBrowserosDir()}`)
}

export function getMemoryDir(): string {
  return join(getBrowserosDir(), PATHS.MEMORY_DIR_NAME)
}

export function getSessionsDir(): string {
  return join(getBrowserosDir(), PATHS.SESSIONS_DIR_NAME)
}

export function getSoulPath(): string {
  return join(getBrowserosDir(), PATHS.SOUL_FILE_NAME)
}

export function getCoreMemoryPath(): string {
  return join(getMemoryDir(), PATHS.CORE_MEMORY_FILE_NAME)
}

export function getSkillsDir(): string {
  return join(getBrowserosDir(), PATHS.SKILLS_DIR_NAME)
}

export function getBuiltinSkillsDir(): string {
  return join(getSkillsDir(), PATHS.BUILTIN_DIR_NAME)
}

export function getOpenClawDir(): string {
  return join(getVmStateDir(), PATHS.OPENCLAW_DIR_NAME)
}

export function getLegacyOpenClawDir(): string {
  return join(getBrowserosDir(), PATHS.OPENCLAW_DIR_NAME)
}

export function getCacheDir(): string {
  return join(getBrowserosDir(), PATHS.CACHE_DIR_NAME)
}

/** Returns the durable SQLite database path for local BrowserOS server state. */
export function getDbPath(): string {
  return join(getBrowserosDir(), PATHS.DB_DIR_NAME, PATHS.DB_FILE_NAME)
}

export function getVmCacheDir(): string {
  return join(getCacheDir(), 'vm')
}

export function getLimaHomeDir(): string {
  return join(getBrowserosDir(), 'lima')
}

export function getVmStateDir(): string {
  return join(getBrowserosDir(), 'vm')
}

export function getVmDisksDir(): string {
  return getVmCacheDir()
}

export function getLazyMonitoringDir(): string {
  return join(getBrowserosDir(), 'lazy-monitoring')
}

export function getLazyMonitoringRunsDir(): string {
  return join(getLazyMonitoringDir(), 'runs')
}

export function getLazyMonitoringRunDir(runId: string): string {
  return join(getLazyMonitoringRunsDir(), runId)
}

export function getServerConfigPath(): string {
  return join(getBrowserosDir(), PATHS.SERVER_CONFIG_FILE_NAME)
}

export async function writeServerConfig(
  config: ServerDiscoveryConfig,
): Promise<void> {
  await writeFile(getServerConfigPath(), `${JSON.stringify(config, null, 2)}\n`)
}

export function removeServerConfigSync(): void {
  try {
    unlinkSync(getServerConfigPath())
  } catch {
    // File may not exist or already be removed
  }
}

export async function ensureBrowserosDir(): Promise<void> {
  logDevelopmentBrowserosDir()
  await ensureDirectory(getMemoryDir())
  await ensureDirectory(getSkillsDir())
  await ensureDirectory(getBuiltinSkillsDir())
  await ensureDirectory(getSessionsDir())
  await ensureDirectory(getLazyMonitoringRunsDir())
  await ensureDirectory(getVmDisksDir())
}

export async function cleanOldSessions(): Promise<void> {
  const sessionsDir = getSessionsDir()
  let entries: string[]
  try {
    entries = await readdir(sessionsDir)
  } catch {
    return
  }

  const cutoff = Date.now() - PATHS.SESSION_RETENTION_DAYS * 24 * 60 * 60 * 1000
  let removed = 0

  for (const entry of entries) {
    const entryPath = join(sessionsDir, entry)
    try {
      const info = await stat(entryPath)
      if (info.isDirectory() && info.mtimeMs < cutoff) {
        await rm(entryPath, { recursive: true })
        removed++
      }
    } catch {
      // skip entries that were already removed or inaccessible
    }
  }

  if (removed > 0) {
    logger.info(`Cleaned ${removed} stale session directories`)
  }
}
