import { constants } from 'node:fs'
import { open, stat } from 'node:fs/promises'
import { tool } from 'ai'
import { z } from 'zod'
import {
  DEFAULT_GREP_LIMIT,
  executeWithMetrics,
  GREP_MAX_LINE_LENGTH,
  isBinaryPath,
  MAX_GREP_FILE_SIZE,
  resolveWorkspacePath,
  toModelOutput,
  truncateLine,
  walkFiles,
} from './utils'

const TOOL_NAME = 'filesystem_grep'

interface GrepMatch {
  file: string
  lineNum: number
  line: string
  isMatch: boolean
}

const GREP_FILE_OPEN_FLAGS =
  constants.O_RDONLY |
  (typeof constants.O_NOFOLLOW === 'number' ? constants.O_NOFOLLOW : 0)

/** Reads grep input from a descriptor so final-component symlinks are rejected. */
async function readGrepFileContent(
  filePath: string,
  maxBytes?: number,
): Promise<string | null> {
  let handle: Awaited<ReturnType<typeof open>>
  try {
    handle = await open(filePath, GREP_FILE_OPEN_FLAGS)
  } catch {
    return null
  }

  try {
    const fileStat = await handle.stat()
    if (!fileStat.isFile()) return null
    if (maxBytes !== undefined && fileStat.size > maxBytes) return null
    return await handle.readFile({ encoding: 'utf-8' })
  } catch {
    return null
  } finally {
    await handle.close()
  }
}

function searchFileLines(
  filePath: string,
  lines: string[],
  regex: RegExp,
  context: number,
  maxMatches: number,
): GrepMatch[] {
  const matchIndices: number[] = []
  for (let i = 0; i < lines.length && matchIndices.length < maxMatches; i++) {
    if (regex.test(lines[i])) {
      matchIndices.push(i)
    }
  }

  if (matchIndices.length === 0) return []

  const included = new Set<number>()
  for (const idx of matchIndices) {
    for (
      let i = Math.max(0, idx - context);
      i <= Math.min(lines.length - 1, idx + context);
      i++
    ) {
      included.add(i)
    }
  }

  const matchSet = new Set(matchIndices)
  return [...included]
    .sort((a, b) => a - b)
    .map((i) => ({
      file: filePath,
      lineNum: i + 1,
      line: lines[i],
      isMatch: matchSet.has(i),
    }))
}

function formatMatches(matches: GrepMatch[], context: number): string {
  const lines: string[] = []
  let prevFile = ''
  let prevLineNum = -1

  for (const m of matches) {
    if (m.file !== prevFile) {
      if (lines.length > 0) lines.push('')
      prevFile = m.file
      prevLineNum = -1
    }

    if (context > 0 && prevLineNum !== -1 && m.lineNum > prevLineNum + 1) {
      lines.push('--')
    }

    const sep = m.isMatch ? ':' : '-'
    const displayLine = truncateLine(m.line, GREP_MAX_LINE_LENGTH)
    lines.push(`${m.file}${sep}${m.lineNum}${sep}${displayLine}`)
    prevLineNum = m.lineNum
  }

  return lines.join('\n')
}

export function createGrepTool(cwd: string) {
  return tool({
    description:
      'Search file contents using a regular expression. Returns matching lines with file paths and line numbers. Searches recursively, skipping binary files and common build directories (node_modules, .git, dist, etc.).',
    inputSchema: z.object({
      pattern: z
        .string()
        .describe(
          'Search pattern (regex by default, or literal string if literal=true)',
        ),
      path: z
        .string()
        .optional()
        .describe(
          'Directory or file to search relative to the selected workspace',
        ),
      glob: z
        .string()
        .optional()
        .describe('Filter files by glob pattern (e.g., "*.ts", "*.{js,jsx}")'),
      ignore_case: z.boolean().optional().describe('Case-insensitive search'),
      literal: z
        .boolean()
        .optional()
        .describe('Treat pattern as a literal string, not regex'),
      context: z
        .number()
        .optional()
        .describe('Lines of context before and after each match'),
      limit: z
        .number()
        .optional()
        .describe(`Maximum matches to return (default: ${DEFAULT_GREP_LIMIT})`),
    }),
    execute: (params) =>
      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: grep tool has many output mode and filtering branches
      executeWithMetrics(TOOL_NAME, async () => {
        const searchPath = await resolveWorkspacePath(cwd, params.path || '.')
        const limit = params.limit || DEFAULT_GREP_LIMIT
        const context = params.context || 0

        const escapedPattern = params.literal
          ? params.pattern.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
          : params.pattern

        let regex: RegExp
        try {
          regex = new RegExp(escapedPattern, params.ignore_case ? 'i' : '')
        } catch (_e) {
          return {
            text: `Invalid regex pattern: ${params.pattern}`,
            isError: true,
          }
        }

        let globMatcher: InstanceType<typeof Bun.Glob> | null = null
        if (params.glob) {
          let effectiveGlob = params.glob
          if (!effectiveGlob.includes('/') && !effectiveGlob.includes('**')) {
            effectiveGlob = `**/${effectiveGlob}`
          }
          globMatcher = new Bun.Glob(effectiveGlob)
        }

        let pathStat: Awaited<ReturnType<typeof stat>>
        try {
          pathStat = await stat(searchPath)
        } catch {
          return {
            text: `Path not found: ${params.path || '.'}`,
            isError: true,
          }
        }
        const allMatches: GrepMatch[] = []
        let totalMatchCount = 0

        if (pathStat.isFile()) {
          const content = await readGrepFileContent(
            searchPath,
            MAX_GREP_FILE_SIZE,
          )
          if (content === null) {
            return {
              text: `Unable to read file: ${params.path || searchPath}`,
              isError: true,
            }
          }
          const lines = content.split('\n')
          const relPath = params.path || searchPath
          const fileMatches = searchFileLines(
            relPath,
            lines,
            regex,
            context,
            limit,
          )
          allMatches.push(...fileMatches)
          totalMatchCount = fileMatches.filter((m) => m.isMatch).length
        } else {
          for await (const file of walkFiles(searchPath, searchPath)) {
            const relPath = file.path
            if (isBinaryPath(relPath)) continue
            if (globMatcher && !globMatcher.match(relPath)) continue

            const content = await readGrepFileContent(
              file.realPath,
              MAX_GREP_FILE_SIZE,
            )
            if (content === null) continue

            const lines = content.split('\n')
            const remaining = limit - totalMatchCount
            if (remaining <= 0) break

            const fileMatches = searchFileLines(
              relPath,
              lines,
              regex,
              context,
              remaining,
            )
            const matchCount = fileMatches.filter((m) => m.isMatch).length
            totalMatchCount += matchCount
            allMatches.push(...fileMatches)

            if (totalMatchCount >= limit) break
          }
        }

        if (allMatches.length === 0) {
          return { text: 'No matches found.' }
        }

        let result = formatMatches(allMatches, context)
        if (totalMatchCount >= limit) {
          result += `\n\n(Showing first ${limit} matches. Use limit=${limit * 2} to see more.)`
        }

        return { text: result }
      }),
    toModelOutput,
  })
}
