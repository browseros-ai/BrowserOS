import { readFile } from 'node:fs/promises'
import { extname } from 'node:path'
import { tool } from 'ai'
import { z } from 'zod'
import { wrapUntrusted } from '../browser/trust-boundary'
import {
  executeWithMetrics,
  type FilesystemToolResult,
  IMAGE_EXTENSIONS,
  IMAGE_MIME_TYPES,
  isBrowserosStatePath,
  MAX_READ_CHARS,
  MAX_READ_LINES,
  resolveBrowserToolOutputPath,
  resolveWorkspacePath,
  toModelOutput,
} from './utils'

const TOOL_NAME = 'filesystem_read'

export interface ReadToolOptions {
  allowedOutputPaths?: ReadonlySet<string>
  requireAllowedOutputPath?: boolean
}

function createImageResult(
  path: string,
  ext: string,
  buffer: Buffer<ArrayBuffer>,
) {
  const mimeType = IMAGE_MIME_TYPES[ext] || 'application/octet-stream'
  return {
    text: `Image: ${path} (${buffer.byteLength} bytes)`,
    images: [{ data: buffer.toString('base64'), mimeType }],
  }
}

function getStartIndex(offset?: number): number {
  return offset ? Math.max(0, offset - 1) : 0
}

function getSelectedLines(
  allLines: string[],
  startIdx: number,
  limit?: number,
): string[] {
  if (limit !== undefined && limit <= 0) {
    throw new Error('filesystem_read limit must be greater than 0.')
  }

  if (limit !== undefined && limit > MAX_READ_LINES) {
    throw new Error(
      `filesystem_read accepts at most ${MAX_READ_LINES} lines per call. Retry with a smaller limit.`,
    )
  }

  const remaining = allLines.slice(startIdx)
  if (limit !== undefined && limit < remaining.length) {
    return remaining.slice(0, limit)
  }
  return remaining
}

function validateSelectedRange(selected: string[], startIdx: number): void {
  const startLineNum = startIdx + 1
  const endLineNum = startIdx + selected.length

  if (selected.length > MAX_READ_LINES) {
    throw new Error(
      `Requested lines ${startLineNum}-${endLineNum} exceed the ${MAX_READ_LINES}-line limit for filesystem_read. Retry with offset and limit=${MAX_READ_LINES} or smaller.`,
    )
  }
}

function formatReadResult(args: {
  selected: string[]
  startIdx: number
  totalLines: number
  limit?: number
}): FilesystemToolResult {
  const startLineNum = args.startIdx + 1
  const endLineNum = args.startIdx + args.selected.length
  const width = String(endLineNum).length
  const numbered = args.selected
    .map((line, i) => {
      const num = String(args.startIdx + i + 1).padStart(width)
      return `${num} | ${line}`
    })
    .join('\n')

  let text = numbered
  if (args.limit && endLineNum < args.totalLines) {
    text += `\n\n(${args.totalLines - endLineNum} more lines in file. Use offset=${endLineNum + 1} to continue reading.)`
  } else if (args.startIdx > 0) {
    text += `\n\n(Showing lines ${startLineNum}-${endLineNum} of ${args.totalLines})`
  }

  if (text.length > MAX_READ_CHARS) {
    throw new Error(
      `Requested lines ${startLineNum}-${endLineNum} produce ${text.length} characters in the response, above the ${MAX_READ_CHARS}-character limit for filesystem_read. Retry with a smaller limit or a later offset.`,
    )
  }

  return { text }
}

const NO_WORKSPACE_READ_ERROR =
  'No workspace selected. filesystem_read can only read BrowserOS-generated tool output files by absolute path.'

function assertAllowedGeneratedOutputPath(
  resolvedPath: string,
  allowedOutputPaths: ReadonlySet<string>,
): void {
  if (!allowedOutputPaths.has(resolvedPath)) {
    throw new Error(
      'filesystem_read can only read BrowserOS-generated tool output files returned in this session.',
    )
  }
}

async function resolveGeneratedOutputPath(
  inputPath: string,
  allowedOutputPaths: ReadonlySet<string>,
): Promise<string> {
  if (!(await isBrowserosStatePath(inputPath))) {
    throw new Error(NO_WORKSPACE_READ_ERROR)
  }
  const resolved = await resolveBrowserToolOutputPath(inputPath)
  assertAllowedGeneratedOutputPath(resolved, allowedOutputPaths)
  return resolved
}

async function resolveReadPath(
  cwd: string | undefined,
  inputPath: string,
  allowedOutputPaths: ReadonlySet<string>,
  requireAllowedOutputPath: boolean,
): Promise<string> {
  if (!cwd)
    return await resolveGeneratedOutputPath(inputPath, allowedOutputPaths)

  try {
    return await resolveWorkspacePath(cwd, inputPath)
  } catch (error) {
    if (error instanceof Error && (await isBrowserosStatePath(inputPath))) {
      if (requireAllowedOutputPath) {
        return await resolveGeneratedOutputPath(inputPath, allowedOutputPaths)
      }
      return await resolveBrowserToolOutputPath(inputPath)
    }
    throw error
  }
}

/** Creates the read tool for workspace files, or generated browser outputs when no workspace exists. */
export function createReadTool(cwd?: string, options: ReadToolOptions = {}) {
  const allowedOutputPaths = options.allowedOutputPaths ?? new Set<string>()
  const supportsGeneratedOutputs = Boolean(options.allowedOutputPaths)

  return tool({
    description: cwd
      ? `Read a file from the filesystem. Returns text content with line numbers, or image data for image files. Text reads are limited to ${MAX_READ_LINES} lines and ${MAX_READ_CHARS} characters per call. Use offset and limit to paginate through large files.${supportsGeneratedOutputs ? ' Also accepts absolute BrowserOS-generated output file paths returned by browser tools.' : ''}`
      : `Read BrowserOS-generated tool output files by absolute path. Returns text content with line numbers, or image data for image files. Text reads are limited to ${MAX_READ_LINES} lines and ${MAX_READ_CHARS} characters per call. Use offset and limit to paginate through large files.`,
    inputSchema: z.object({
      path: z
        .string()
        .describe(
          cwd
            ? `File path relative to the selected workspace${supportsGeneratedOutputs ? ', or an absolute BrowserOS-generated output path returned by a browser tool' : ''}`
            : 'Absolute BrowserOS-generated tool output path returned by a browser tool',
        ),
      offset: z
        .number()
        .optional()
        .describe('Starting line number (1-indexed)'),
      limit: z
        .number()
        .int()
        .positive()
        .optional()
        .describe('Maximum number of lines to read'),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const resolved = await resolveReadPath(
          cwd,
          params.path,
          allowedOutputPaths,
          options.requireAllowedOutputPath ?? false,
        )
        const ext = extname(resolved).toLowerCase()

        if (IMAGE_EXTENSIONS.has(ext)) {
          const buffer = await readFile(resolved)
          return createImageResult(params.path, ext, buffer)
        }

        const content = await readFile(resolved, 'utf-8')
        const allLines = content.split('\n')
        const totalLines = allLines.length

        const startIdx = getStartIndex(params.offset)
        if (startIdx >= totalLines) {
          return {
            text: `File has ${totalLines} lines. Offset ${params.offset} is beyond end of file.`,
          }
        }

        const selected = getSelectedLines(allLines, startIdx, params.limit)
        validateSelectedRange(selected, startIdx)
        const result = formatReadResult({
          selected,
          startIdx,
          totalLines,
          limit: params.limit,
        })
        if (!cwd) {
          return {
            ...result,
            text: wrapUntrusted(result.text, params.path),
          }
        }
        return result
      }),
    toModelOutput,
  })
}
