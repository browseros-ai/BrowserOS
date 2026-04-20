import { unlinkSync } from 'node:fs'
import { mkdir, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import type { ServerDiscoveryConfig } from '@browseros/shared/types/server-config'
import { logger } from './logger'

const DEV_BROWSEROS_DIR_NAME = '.browseros-dev'

export function getBrowserosDir(): string {
  const dirName =
    process.env.NODE_ENV === 'development'
      ? DEV_BROWSEROS_DIR_NAME
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
  return join(getBrowserosDir(), PATHS.OPENCLAW_DIR_NAME)
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
  await mkdir(getMemoryDir(), { recursive: true })
  await mkdir(getSkillsDir(), { recursive: true })
  await mkdir(getBuiltinSkillsDir(), { recursive: true })
  await mkdir(getSessionsDir(), { recursive: true })
  await mkdir(getLazyMonitoringRunsDir(), { recursive: true })
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
