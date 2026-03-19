import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tool } from 'ai'
import Fuse from 'fuse.js'
import { z } from 'zod'
import { getMemoryDir } from '../../lib/browseros-dir'
import { executeWithMetrics, toModelOutput } from '../filesystem/utils'

const TOOL_NAME = 'memory_search'

interface MemoryEntry {
  source: string
  content: string
}

async function loadMemoryEntries(): Promise<MemoryEntry[]> {
  const memoryDir = getMemoryDir()
  let files: string[]
  try {
    files = await readdir(memoryDir)
  } catch {
    return []
  }

  const mdFiles = files.filter((f) => f.endsWith('.md'))

  const entries: MemoryEntry[] = []
  for (const file of mdFiles) {
    try {
      const content = await readFile(join(memoryDir, file), 'utf-8')

      // Section-level entries (## delimited blocks)
      const sections = content.split(/^## /m).filter(Boolean)
      for (const section of sections) {
        entries.push({ source: file, content: `## ${section}`.trim() })
      }

      // Line-level entries (individual non-empty lines)
      for (const line of content.split('\n')) {
        const trimmed = line.trim()
        if (trimmed && !trimmed.startsWith('#')) {
          entries.push({ source: file, content: trimmed })
        }
      }
    } catch {
      // skip unreadable files
    }
  }
  return entries
}

export function createMemorySearchTool() {
  return tool({
    description:
      'Search all memories (both core and daily) using fuzzy matching. Pass multiple keywords for broader recall — each keyword is searched independently and results are merged by best relevance.',
    inputSchema: z.object({
      keywords: z
        .array(z.string())
        .min(1)
        .describe(
          'Search keywords/terms. Use multiple to cast a wider net (e.g. ["user name", "preferences", "location"]).',
        ),
    }),
    execute: (params) =>
      executeWithMetrics(TOOL_NAME, async () => {
        const entries = await loadMemoryEntries()
        if (entries.length === 0) {
          return { text: 'No memories found.' }
        }

        const fuse = new Fuse(entries, {
          keys: ['content'],
          threshold: 0.4,
          includeScore: true,
        })

        const bestScores = new Map<
          MemoryEntry,
          { score: number; keyword: string }
        >()
        for (const keyword of params.keywords) {
          for (const r of fuse.search(keyword)) {
            const score = r.score ?? 1
            const existing = bestScores.get(r.item)
            if (!existing || score < existing.score) {
              bestScores.set(r.item, { score, keyword })
            }
          }
        }

        if (bestScores.size === 0) {
          return {
            text: `No memories matching [${params.keywords.join(', ')}] found.`,
          }
        }

        const sorted = [...bestScores.entries()].sort(
          (a, b) => a[1].score - b[1].score,
        )

        // Deduplicate: if a line-level match is already contained in a
        // higher-ranked section-level match from the same file, skip it.
        const seen = new Set<string>()
        const deduped: typeof sorted = []
        for (const item of sorted) {
          const key = `${item[0].source}::${item[0].content}`
          if (seen.has(key)) continue
          seen.add(key)
          deduped.push(item)
        }

        const formatted = deduped
          .slice(0, 10)
          .map(([entry, { score }]) => {
            const relevance = (1 - score).toFixed(2)
            return `[${entry.source}] (relevance: ${relevance})\n${entry.content}`
          })
          .join('\n\n---\n\n')

        return { text: formatted }
      }),
    toModelOutput,
  })
}
