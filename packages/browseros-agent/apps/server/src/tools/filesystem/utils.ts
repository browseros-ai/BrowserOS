import type { Dirent } from 'node:fs'
import { readdir, realpath } from 'node:fs/promises'
import { join, relative } from 'node:path'
import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { isPathInside } from './path-boundary'

const MAX_LINES = 2000
const MAX_BYTES = 50 * 1024
export const GREP_MAX_LINE_LENGTH = 500
export const DEFAULT_GREP_LIMIT = 100
export const DEFAULT_FIND_LIMIT = 1000
export const DEFAULT_LS_LIMIT = 500
export const DEFAULT_BASH_TIMEOUT = 120
export const MAX_GREP_FILE_SIZE = 2 * 1024 * 1024
export const MAX_READ_LINES = TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES
export const MAX_READ_CHARS = TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS

export {
  isBrowserosStatePath,
  resolveBrowserToolOutputPath,
  resolveWorkspacePath,
  resolveWorkspacePathFromRoot,
  resolveWorkspaceRoot,
  resolveWorkspaceWritePath,
} from './path-boundary'

export interface FilesystemToolResult {
  text: string
  isError?: boolean
  images?: Array<{ data: string; mimeType: string }>
}

export interface TruncationResult {
  content: string
  truncated: boolean
  totalLines: number
  keptLines: number
}

export function truncateHead(
  content: string,
  maxLines = MAX_LINES,
  maxBytes = MAX_BYTES,
): TruncationResult {
  const lines = content.split('\n')
  const totalLines = lines.length
  const kept: string[] = []
  let bytes = 0

  for (const line of lines) {
    const lineBytes = Buffer.byteLength(line, 'utf-8') + 1
    if (kept.length >= maxLines || bytes + lineBytes > maxBytes) {
      return {
        content: kept.join('\n'),
        truncated: true,
        totalLines,
        keptLines: kept.length,
      }
    }
    kept.push(line)
    bytes += lineBytes
  }

  return {
    content: kept.join('\n'),
    truncated: false,
    totalLines,
    keptLines: kept.length,
  }
}

export function truncateTail(
  content: string,
  maxLines = MAX_LINES,
  maxBytes = MAX_BYTES,
): TruncationResult {
  const lines = content.split('\n')
  const totalLines = lines.length
  const kept: string[] = []
  let bytes = 0

  for (let i = lines.length - 1; i >= 0; i--) {
    const lineBytes = Buffer.byteLength(lines[i], 'utf-8') + 1
    if (kept.length >= maxLines || bytes + lineBytes > maxBytes) {
      return {
        content: kept.reverse().join('\n'),
        truncated: true,
        totalLines,
        keptLines: kept.length,
      }
    }
    kept.push(lines[i])
    bytes += lineBytes
  }

  return {
    content: kept.reverse().join('\n'),
    truncated: false,
    totalLines,
    keptLines: kept.length,
  }
}

export function truncateLine(
  line: string,
  maxChars = GREP_MAX_LINE_LENGTH,
): string {
  if (line.length <= maxChars) return line
  return `${line.slice(0, maxChars)} [truncated]`
}

const IGNORED_DIRS = new Set([
  'node_modules',
  '.git',
  'dist',
  'build',
  '.next',
  'coverage',
  '__pycache__',
  '.venv',
  'venv',
  '.tox',
  'target',
  '.gradle',
  '.idea',
  '.vscode',
  '.cache',
  '.turbo',
  '.output',
  '.nuxt',
  '.svelte-kit',
  '.parcel-cache',
  '.angular',
  '.expo',
  '.yarn',
  '.pnp',
])

const BINARY_EXTENSIONS = new Set([
  '.png',
  '.jpg',
  '.jpeg',
  '.gif',
  '.webp',
  '.bmp',
  '.ico',
  '.woff',
  '.woff2',
  '.ttf',
  '.eot',
  '.otf',
  '.pdf',
  '.doc',
  '.docx',
  '.xls',
  '.xlsx',
  '.ppt',
  '.pptx',
  '.zip',
  '.tar',
  '.gz',
  '.bz2',
  '.rar',
  '.7z',
  '.xz',
  '.exe',
  '.dll',
  '.so',
  '.dylib',
  '.bin',
  '.o',
  '.a',
  '.mp3',
  '.mp4',
  '.avi',
  '.mov',
  '.wav',
  '.flac',
  '.ogg',
  '.sqlite',
  '.db',
  '.wasm',
  '.class',
  '.pyc',
])

export function isBinaryPath(filePath: string): boolean {
  const dot = filePath.lastIndexOf('.')
  if (dot === -1) return false
  return BINARY_EXTENSIONS.has(filePath.slice(dot).toLowerCase())
}

export const IMAGE_EXTENSIONS = new Set([
  '.png',
  '.jpg',
  '.jpeg',
  '.gif',
  '.webp',
  '.bmp',
  '.svg',
  '.ico',
])

export const IMAGE_MIME_TYPES: Record<string, string> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.bmp': 'image/bmp',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
}

export interface WalkFileEntry {
  path: string
  realPath: string
}

/** Walks visible files while preserving both display and canonical read paths. */
export async function* walkFiles(
  dir: string,
  baseDir: string,
  boundaryRoot?: string,
): AsyncGenerator<WalkFileEntry> {
  const root = boundaryRoot ?? (await realpath(baseDir))
  let entries: Dirent[]
  try {
    entries = (await readdir(dir, { withFileTypes: true })) as Dirent[]
  } catch {
    return
  }

  for (const entry of entries) {
    const fullPath = join(dir, entry.name as string)
    if (entry.isDirectory()) {
      if (IGNORED_DIRS.has(entry.name as string)) continue
      yield* walkFiles(fullPath, baseDir, root)
    } else if (entry.isFile() || entry.isSymbolicLink()) {
      const canonical = await realpath(fullPath).catch(() => null)
      if (!canonical || !isPathInside(root, canonical)) continue
      yield {
        path: relative(baseDir, fullPath),
        realPath: canonical,
      }
    }
  }
}

export function toModelOutput({
  output,
}: {
  output: unknown
  toolCallId: string
  input: unknown
}) {
  const result = output as FilesystemToolResult
  if (result.isError) {
    return { type: 'error-text' as const, value: result.text }
  }
  if (result.images?.length) {
    return {
      type: 'content' as const,
      value: [
        { type: 'text' as const, text: result.text },
        ...result.images.map((img) => ({
          type: 'media' as const,
          data: img.data,
          mediaType: img.mimeType,
        })),
      ],
    }
  }
  return { type: 'text' as const, value: result.text || 'Success' }
}

export function executeWithMetrics(
  toolName: string,
  fn: () => Promise<FilesystemToolResult>,
): Promise<FilesystemToolResult> {
  const startTime = performance.now()
  return fn().then(
    (result) => {
      metrics.log('tool_executed', {
        tool_name: toolName,
        duration_ms: Math.round(performance.now() - startTime),
        success: !result.isError,
        source: 'chat',
      })
      return result
    },
    (error) => {
      const errorText = error instanceof Error ? error.message : String(error)
      logger.error('Filesystem tool execution failed', {
        tool: toolName,
        error: errorText,
      })
      metrics.log('tool_executed', {
        tool_name: toolName,
        duration_ms: Math.round(performance.now() - startTime),
        success: false,
        error_message: errorText,
        source: 'chat',
      })
      return { text: errorText, isError: true }
    },
  )
}

export function stripBom(content: string): {
  content: string
  hasBom: boolean
} {
  if (content.charCodeAt(0) === 0xfeff) {
    return { content: content.slice(1), hasBom: true }
  }
  return { content, hasBom: false }
}

export function detectLineEnding(content: string): '\r\n' | '\r' | '\n' {
  const crlfIdx = content.indexOf('\r\n')
  if (crlfIdx !== -1) return '\r\n'
  const crIdx = content.indexOf('\r')
  if (crIdx !== -1) return '\r'
  return '\n'
}

export function normalizeToLF(content: string): string {
  return content.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
}

export function restoreLineEndings(
  content: string,
  lineEnding: string,
): string {
  if (lineEnding === '\n') return content
  return content.replace(/\n/g, lineEnding)
}
