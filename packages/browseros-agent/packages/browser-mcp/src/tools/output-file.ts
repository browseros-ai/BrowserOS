import { AsyncLocalStorage } from 'node:async_hooks'
import { randomUUID } from 'node:crypto'
import { join } from 'node:path'
import {
  getToolOutputDir,
  writeToolOutputBinaryFile,
  writeToolOutputFile,
} from '../tool-output-dir'

export interface BrowserOutputFileAccess {
  readonly paths: ReadonlySet<string>
  record(path: string): void
}

const outputFileAccessStorage = new AsyncLocalStorage<BrowserOutputFileAccess>()

/** Creates per-agent access state for generated browser output files. */
export function createBrowserOutputFileAccess(): BrowserOutputFileAccess {
  const paths = new Set<string>()
  return {
    paths,
    record(path: string) {
      paths.add(path)
    },
  }
}

/** Runs browser work while registering generated output files for later readback. */
export function withBrowserOutputFileAccess<T>(
  access: BrowserOutputFileAccess | undefined,
  run: () => T,
): T {
  if (!access) return run()
  return outputFileAccessStorage.run(access, run)
}

/** Allows the current agent to read a browser-generated output path later. */
export function recordBrowserOutputFile(path: string): void {
  outputFileAccessStorage.getStore()?.record(path)
}

function sanitizeSegment(value: string): string {
  const sanitized = value.replace(/[^a-z0-9_-]+/gi, '-').replace(/^-+|-+$/g, '')
  return sanitized || 'browser-tool-output'
}

function uniqueOutputPath(
  outputDir: string,
  toolName: string,
  extension: string,
): string {
  return join(
    outputDir,
    `${sanitizeSegment(toolName)}-${Date.now()}-${randomUUID()}.${
      sanitizeSegment(extension) || 'txt'
    }`,
  )
}

export async function writeTempToolOutputFile(args: {
  toolName: string
  extension: string
  content: string
}): Promise<string> {
  const outputDir = await getToolOutputDir()
  const filePath = uniqueOutputPath(outputDir, args.toolName, args.extension)
  await writeToolOutputFile(filePath, args.content)
  recordBrowserOutputFile(filePath)
  return filePath
}

export async function writeTempToolOutputBinaryFile(args: {
  toolName: string
  extension: string
  content: Uint8Array
}): Promise<string> {
  const outputDir = await getToolOutputDir()
  const filePath = uniqueOutputPath(outputDir, args.toolName, args.extension)
  await writeToolOutputBinaryFile(filePath, args.content)
  recordBrowserOutputFile(filePath)
  return filePath
}
