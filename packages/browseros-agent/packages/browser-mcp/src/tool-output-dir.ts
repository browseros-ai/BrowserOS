import { chmod, lstat, mkdir, realpath, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'

export const TOOL_OUTPUT_DIR_MODE = 0o700
export const TOOL_OUTPUT_FILE_MODE = 0o600

export function getBrowserosDir(): string {
  const override = process.env.BROWSEROS_DIR?.trim()
  if (override) return override
  const dirName =
    process.env.NODE_ENV === 'development'
      ? PATHS.DEV_BROWSEROS_DIR_NAME
      : PATHS.BROWSEROS_DIR_NAME
  return join(homedir(), dirName)
}

/** Returns the ready-to-use directory for large generated browser outputs. */
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

/** Writes a generated browser output file with private owner-only permissions. */
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

/** Writes binary browser output with private owner-only permissions. */
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
