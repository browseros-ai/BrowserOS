import { tool } from 'ai'
import { z } from 'zod'
import { getCoreMemoryPath } from '../../lib/browseros-dir'
import { executeWithMetrics, toModelOutput } from '../filesystem/utils'

const TOOL_NAME = 'memory_save_core'

export function createSaveCoreTool() {
  return tool({
    description:
      'DEPRECATED: Use memory_update_core instead. This tool overwrites the entire core memory file and risks data loss. If you must use it, always call memory_read_core first, merge your changes into the existing content, then save the full result.',
    inputSchema: z.object({
      content: z.string().describe('The full core memory content to save'),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        await Bun.write(getCoreMemoryPath(), params.content)
        return { text: 'Core memories updated.' }
      }),
    toModelOutput,
  })
}
