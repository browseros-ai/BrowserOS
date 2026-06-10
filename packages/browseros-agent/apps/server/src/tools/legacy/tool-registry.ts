import type { ToolDefinition } from './framework'

class ToolRegistry {
  readonly tools: Map<string, ToolDefinition>

  constructor(tools: ToolDefinition[]) {
    this.tools = new Map()
    for (const tool of tools) {
      if (this.tools.has(tool.name)) {
        throw new Error(`Duplicate tool name: "${tool.name}"`)
      }
      this.tools.set(tool.name, tool)
    }
  }

  all(): ToolDefinition[] {
    return [...this.tools.values()]
  }

  get(name: string): ToolDefinition | undefined {
    return this.tools.get(name)
  }

  names(): string[] {
    return [...this.tools.keys()]
  }
}

export function createRegistry(tools: ToolDefinition[]): ToolRegistry {
  return new ToolRegistry(tools)
}
