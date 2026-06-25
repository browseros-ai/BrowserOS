import { unlinkSync } from 'node:fs'
import {
  chmod,
  lstat,
  mkdir,
  readdir,
  realpath,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import type { ServerDiscoveryConfig } from '@browseros/shared/types/server-config'
import { logger } from './logger'

export const TOOL_OUTPUT_DIR_MODE = 0o700
export const TOOL_OUTPUT_FILE_MODE = 0o600

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

export function getSessionsDir(): string {
  return join(getBrowserosDir(), PATHS.SESSIONS_DIR_NAME)
}

export function getCacheDir(): string {
  return join(getBrowserosDir(), PATHS.CACHE_DIR_NAME)
}

/** Returns the ready-to-use directory for large generated tool outputs. */
export async function getToolOutputDir(): Promise<string> {
  const outputDirPath = join(getBrowserosDir(), 'tool-output')
  await mkdir(outputDirPath, {
    recursive: true,
    mode: TOOL_OUTPUT_DIR_MODE,
  })
  const info = await lstat(outputDirPath)
  if (!info.isDirectory() || info.isSymbolicLink()) {
    throw new Error('BrowserOS tool output directory must be a real directory.')
  }
  const outputDir = await realpath(outputDirPath)
  await chmod(outputDir, TOOL_OUTPUT_DIR_MODE)
  return outputDir
}

/** Writes a generated tool output file with private owner-only permissions. */
export async function writeToolOutputFile(
  filePath: string,
  content: string,
): Promise<void> {
  await writeFile(filePath, content, {
    encoding: 'utf-8',
    flag: 'wx',
    mode: TOOL_OUTPUT_FILE_MODE,
  })
  await chmod(filePath, TOOL_OUTPUT_FILE_MODE)
}

/** Writes binary tool output (PDFs, downloads) with the same owner-only permissions. */
export async function writeToolOutputBinaryFile(
  filePath: string,
  content: Uint8Array,
): Promise<void> {
  await writeFile(filePath, content, {
    flag: 'wx',
    mode: TOOL_OUTPUT_FILE_MODE,
  })
  await chmod(filePath, TOOL_OUTPUT_FILE_MODE)
}

/** Returns the durable SQLite database path for local BrowserOS server state. */
export function getDbPath(): string {
  return join(getBrowserosDir(), PATHS.DB_DIR_NAME, PATHS.DB_FILE_NAME)
}

function getServerConfigPath(): string {
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
    return
  }
}

export async function ensureBrowserosDir(): Promise<void> {
  logDevelopmentBrowserosDir()
  await mkdir(getSessionsDir(), { recursive: true })
  await getToolOutputDir()
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
