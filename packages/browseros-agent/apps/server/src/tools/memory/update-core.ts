import { tool } from 'ai'
import { z } from 'zod'
import { getCoreMemoryPath } from '../../lib/browseros-dir'
import { executeWithMetrics, toModelOutput } from '../filesystem/utils'

const TOOL_NAME = 'memory_update_core'

export function createUpdateCoreTool() {
  return tool({
    description:
      'Add or remove facts from core memory. Handles merging internally — you never need to rewrite the full file. Pass additions to store new facts and/or removals to delete facts by substring match.',
    inputSchema: z.object({
      additions: z
        .array(z.string())
        .optional()
        .describe('New facts to add to core memory. Each string is one fact.'),
      removals: z
        .array(z.string())
        .optional()
        .describe(
          'Facts to remove from core memory. Each string is matched as a case-insensitive substring against existing lines.',
        ),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const corePath = getCoreMemoryPath()
        const file = Bun.file(corePath)

        let existing = ''
        if (await file.exists()) {
          existing = await file.text()
        }

        let lines = existing.split('\n')

        // Remove matching entries
        let removedCount = 0
        if (params.removals?.length) {
          for (const removal of params.removals) {
            const lower = removal.toLowerCase()
            const before = lines.length
            lines = lines.filter((line) => !line.toLowerCase().includes(lower))
            removedCount += before - lines.length
          }
        }

        // Append new facts (skip duplicates)
        let addedCount = 0
        if (params.additions?.length) {
          if (lines.length > 0 && lines[lines.length - 1].trim() !== '') {
            lines.push('')
          }
          for (const fact of params.additions) {
            const factLower = fact.toLowerCase()
            const alreadyExists = lines.some((l) =>
              l.toLowerCase().includes(factLower),
            )
            if (!alreadyExists) {
              lines.push(`- ${fact}`)
              addedCount++
            }
          }
        }

        const result = `${lines.join('\n').trim()}\n`
        await Bun.write(corePath, result)

        return {
          text: `Core memory updated. ${addedCount} fact(s) added, ${removedCount} line(s) removed.`,
        }
      }),
    toModelOutput,
  })
}
