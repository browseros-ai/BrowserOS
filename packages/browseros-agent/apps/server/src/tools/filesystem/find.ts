import { tool } from 'ai'
import { z } from 'zod'
import {
  DEFAULT_FIND_LIMIT,
  executeWithMetrics,
  resolveWorkspacePath,
  resolveWorkspaceRoot,
  toModelOutput,
  walkFiles,
} from './utils'

const TOOL_NAME = 'filesystem_find'

export function createFindTool(cwd: string) {
  return tool({
    description:
      'Find files matching a glob pattern. Searches recursively, skipping common build directories (node_modules, .git, dist, etc.). Returns relative file paths.',
    inputSchema: z.object({
      pattern: z
        .string()
        .describe(
          'Glob pattern (e.g., "*.ts", "**/*.json", "src/**/*.test.ts")',
        ),
      path: z
        .string()
        .optional()
        .describe('Directory within the working directory'),
      limit: z
        .number()
        .optional()
        .describe(`Maximum results (default: ${DEFAULT_FIND_LIMIT})`),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const searchPath = await resolveWorkspacePath(cwd, params.path || '.')
        const workspaceRoot = await resolveWorkspaceRoot(cwd)
        const limit = params.limit || DEFAULT_FIND_LIMIT

        let effectivePattern = params.pattern
        if (
          !effectivePattern.includes('/') &&
          !effectivePattern.includes('**')
        ) {
          effectivePattern = `**/${effectivePattern}`
        }

        const glob = new Bun.Glob(effectivePattern)
        const matches: string[] = []

        for await (const relPath of walkFiles(
          searchPath,
          searchPath,
          workspaceRoot,
        )) {
          if (glob.match(relPath)) {
            matches.push(relPath)
            if (matches.length >= limit) break
          }
        }

        if (matches.length === 0) {
          return { text: `No files matching "${params.pattern}" found.` }
        }

        matches.sort()
        let result = matches.join('\n')
        if (matches.length >= limit) {
          result += `\n\n(Showing first ${limit} results. Use limit=${limit * 2} to see more.)`
        }

        return { text: result }
      }),
    toModelOutput,
  })
}
