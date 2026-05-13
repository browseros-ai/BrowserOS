/**
 * @license
 * Copyright 2025 BrowserOS
 */

import fs from 'node:fs/promises'
import path from 'node:path'

export interface AgentsMdResult {
  path: string
  content: string
  lastModified: number
}

const MAX_FILE_SIZE = 100 * 1024 // 100KB

interface CacheEntry {
  content: string
  lastModified: number
}

export class AgentsMdLoader {
  private allowedDirs: string[]
  private cache = new Map<string, CacheEntry>()

  constructor(registeredPaths: string[]) {
    this.allowedDirs = registeredPaths.map((p) => path.resolve(p))
  }

  async load(workspacePath: string): Promise<AgentsMdResult | null> {
    const resolved = path.resolve(workspacePath)

    // Security: validate resolved path is within an allowed directory
    if (!this.isAllowed(resolved)) {
      return null
    }

    const agentsMdPath = path.join(resolved, 'AGENTS.md')

    // Check cache
    const cached = this.cache.get(agentsMdPath)
    if (cached) {
      const stat = await this.safeStat(agentsMdPath)
      if (stat && stat.mtimeMs === cached.lastModified) {
        return {
          path: agentsMdPath,
          content: cached.content,
          lastModified: cached.lastModified,
        }
      }
    }

    // Stat for size check and mtime
    const stat = await this.safeStat(agentsMdPath)
    if (!stat) return null
    if (stat.size > MAX_FILE_SIZE) return null

    const content = await fs.readFile(agentsMdPath, 'utf-8')

    const result: AgentsMdResult = {
      path: agentsMdPath,
      content,
      lastModified: stat.mtimeMs,
    }

    this.cache.set(agentsMdPath, {
      content,
      lastModified: stat.mtimeMs,
    })

    return result
  }

  async loadMultiple(paths: string[]): Promise<AgentsMdResult[]> {
    const results = await Promise.all(paths.map((p) => this.load(p)))
    return results.filter((r): r is AgentsMdResult => r !== null)
  }

  private isAllowed(resolvedPath: string): boolean {
    return this.allowedDirs.some((dir) => resolvedPath === dir || resolvedPath.startsWith(dir + path.sep))
  }

  private async safeStat(filePath: string): Promise<{ size: number; mtimeMs: number } | null> {
    try {
      const stat = await fs.stat(filePath)
      return { size: stat.size, mtimeMs: stat.mtimeMs }
    } catch {
      return null
    }
  }
}
