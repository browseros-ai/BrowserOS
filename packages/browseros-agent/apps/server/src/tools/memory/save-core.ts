import { tool } from 'ai'
import { z } from 'zod'
import { getCoreMemoryPath } from '../../lib/browseros-dir'
import { executeWithMetrics, toModelOutput } from '../filesystem/utils'

const TOOL_NAME = 'memory_save_core'

export function createSaveCoreTool() {
  return tool({
    description:
      'Legacy compatibility tool for Sup-agent workflows. Overwrites the entire CORE.md content. Prefer memory_update_core for normal usage.',
    inputSchema: z.object({
      content: z.string().describe('The full core memory content to save'),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        await Bun.write(getCoreMemoryPath(), params.content)
        return { text: 'Core memory updated (legacy save).' }
      }),
    toModelOutput,
  })
}
