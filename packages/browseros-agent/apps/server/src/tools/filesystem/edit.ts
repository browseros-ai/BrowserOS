import { readFile, writeFile } from 'node:fs/promises'
import { tool } from 'ai'
import { z } from 'zod'
import {
  detectLineEnding,
  executeWithMetrics,
  normalizeToLF,
  resolveWorkspacePath,
  restoreLineEndings,
  stripBom,
  toModelOutput,
} from './utils'

const TOOL_NAME = 'filesystem_edit'

function countOccurrences(content: string, search: string): number {
  let count = 0
  let pos = 0
  while (true) {
    pos = content.indexOf(search, pos)
    if (pos === -1) break
    count++
    pos += search.length
  }
  return count
}

function fuzzyReplace(
  content: string,
  oldStr: string,
  newStr: string,
): string | null {
  const contentLines = content.split('\n')
  const searchLines = oldStr.split('\n')
  const trimmedSearch = searchLines.map((l) => l.trim())

  let matchCount = 0
  let matchStartLine = -1

  for (let i = 0; i <= contentLines.length - searchLines.length; i++) {
    let allMatch = true
    for (let j = 0; j < searchLines.length; j++) {
      if (contentLines[i + j].trim() !== trimmedSearch[j]) {
        allMatch = false
        break
      }
    }
    if (allMatch) {
      matchCount++
      if (matchCount === 1) matchStartLine = i
    }
  }

  if (matchCount === 0) return null
  if (matchCount > 1) {
    throw new Error(
      `Whitespace-tolerant match found ${matchCount} occurrences. Add more context to make the match unique.`,
    )
  }

  const before = contentLines.slice(0, matchStartLine)
  const after = contentLines.slice(matchStartLine + searchLines.length)
  return [...before, ...newStr.split('\n'), ...after].join('\n')
}

function generateDiff(oldStr: string, newStr: string): string {
  const oldLines = oldStr.split('\n')
  const newLines = newStr.split('\n')
  const lines: string[] = []
  for (const line of oldLines) lines.push(`- ${line}`)
  for (const line of newLines) lines.push(`+ ${line}`)
  return lines.join('\n')
}

export function createEditTool(cwd: string) {
  return tool({
    description:
      'Make a targeted edit to a file by replacing an exact string match. The old_string must match exactly one location in the file. If exact match fails, a whitespace-tolerant match is attempted.',
    inputSchema: z.object({
      path: z.string().describe('File path relative to the selected workspace'),
      old_string: z.string().describe('Exact text to find in the file'),
      new_string: z.string().describe('Replacement text'),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const resolved = await resolveWorkspacePath(cwd, params.path)
        const raw = await readFile(resolved, 'utf-8')

        const { content: noBom, hasBom } = stripBom(raw)
        const lineEnding = detectLineEnding(noBom)
        const content = normalizeToLF(noBom)
        const oldNorm = normalizeToLF(params.old_string)
        const newNorm = normalizeToLF(params.new_string)

        if (oldNorm === newNorm) {
          return {
            text: 'old_string and new_string are identical — no change needed.',
            isError: true,
          }
        }

        let updated: string
        const exactCount = countOccurrences(content, oldNorm)

        if (exactCount === 1) {
          updated = content.replace(oldNorm, newNorm)
        } else if (exactCount > 1) {
          return {
            text: `Found ${exactCount} exact occurrences of old_string. Add more surrounding context to make the match unique.`,
            isError: true,
          }
        } else {
          const fuzzyResult = fuzzyReplace(content, oldNorm, newNorm)
          if (fuzzyResult === null) {
            return {
              text: 'old_string not found in file (exact and whitespace-tolerant match both failed).',
              isError: true,
            }
          }
          updated = fuzzyResult
        }

        let finalContent = restoreLineEndings(updated, lineEnding)
        if (hasBom) finalContent = `\uFEFF${finalContent}`
        await writeFile(resolved, finalContent, 'utf-8')

        const diff = generateDiff(params.old_string, params.new_string)
        return { text: `Applied edit to ${params.path}\n\n${diff}` }
      }),
    toModelOutput,
  })
}
