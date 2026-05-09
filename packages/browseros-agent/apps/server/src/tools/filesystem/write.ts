import { writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { tool } from 'ai'
import { z } from 'zod'
import { ensureDirectory } from '../../lib/ensure-directory'
import { executeWithMetrics, toModelOutput } from './utils'

const TOOL_NAME = 'filesystem_write'

export function createWriteTool(cwd: string) {
  return tool({
    description:
      "Create or overwrite a file. Automatically creates parent directories if they don't exist. Use this to create new files or completely replace file contents.",
    inputSchema: z.object({
      path: z
        .string()
        .describe('File path (relative to working directory or absolute)'),
      content: z.string().describe('Complete file content to write'),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const resolved = resolve(cwd, params.path)
        await ensureDirectory(dirname(resolved))
        await writeFile(resolved, params.content, 'utf-8')
        const bytes = Buffer.byteLength(params.content, 'utf-8')
        return { text: `Wrote ${bytes} bytes to ${params.path}` }
      }),
    toModelOutput,
  })
}
