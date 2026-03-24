import type { ToolSet } from 'ai'
import { createReadCoreTool } from './read-core'
import { createSoulReadTool } from './read-soul'
import { createSaveCoreTool } from './save-core'
import { createMemorySearchTool } from './search'
import { createUpdateCoreTool } from './update-core'
import { createSoulUpdateTool } from './update-soul'
import { createMemoryWriteTool } from './write'

export function buildMemoryToolSet(): ToolSet {
  return {
    memory_search: createMemorySearchTool(),
    memory_write: createMemoryWriteTool(),
    memory_read_core: createReadCoreTool(),
    // Keep legacy name for Sup-agent compatibility.
    memory_save_core: createSaveCoreTool(),
    memory_update_core: createUpdateCoreTool(),
    soul_read: createSoulReadTool(),
    soul_update: createSoulUpdateTool(),
  }
}
