import { readdir, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { tool } from 'ai'
import { z } from 'zod'
import {
  DEFAULT_LS_LIMIT,
  executeWithMetrics,
  resolveSafePath,
  toModelOutput,
} from './utils'

const TOOL_NAME = 'filesystem_ls'

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`
}

export function createLsTool(cwd: string) {
  return tool({
    description:
      'List directory contents. Shows directories (with trailing /) first, then files with sizes. Entries are sorted alphabetically.',
    inputSchema: z.object({
      path: z
        .string()
        .optional()
        .describe('Directory path (default: working directory)'),
      limit: z
        .number()
        .optional()
        .describe(`Maximum entries to return (default: ${DEFAULT_LS_LIMIT})`),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const resolved = resolveSafePath(cwd, params.path || '.')
        const limit = params.limit || DEFAULT_LS_LIMIT
        const entries = await readdir(resolved, { withFileTypes: true })

        const dirs: string[] = []
        const files: Array<{ name: string; size: number }> = []

        for (const entry of entries) {
          if (entry.isDirectory()) {
            dirs.push(entry.name)
          } else {
            try {
              const info = await stat(join(resolved, entry.name))
              files.push({ name: entry.name, size: info.size })
            } catch {
              files.push({ name: entry.name, size: 0 })
            }
          }
        }

        dirs.sort((a, b) =>
          a.localeCompare(b, undefined, { sensitivity: 'base' }),
        )
        files.sort((a, b) =>
          a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
        )

        const lines: string[] = []
        for (const dir of dirs) {
          if (lines.length >= limit) break
          lines.push(`${dir}/`)
        }
        for (const file of files) {
          if (lines.length >= limit) break
          lines.push(`${file.name} (${formatSize(file.size)})`)
        }

        if (lines.length === 0) {
          return { text: '(empty directory)' }
        }

        let result = lines.join('\n')
        const total = dirs.length + files.length
        if (total > limit) {
          result += `\n\n(Showing ${limit} of ${total} entries. Use limit=${limit * 2} to see more.)`
        }

        return { text: result }
      }),
    toModelOutput,
  })
}
