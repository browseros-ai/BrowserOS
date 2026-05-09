import { readdir, readFile, stat } from 'node:fs/promises'
import { basename, resolve } from 'node:path'
import Fuse from 'fuse.js'
import { Hono } from 'hono'
import { getMemoryDir } from '../../lib/browseros-dir'
import { isBinaryPath, walkFiles } from '../../tools/filesystem/utils'

const MAX_RESULTS = 20
const MAX_FILE_SCAN = 2_000
const MAX_FILE_BYTES = 1_000_000
const MAX_CONTENT_CHARS = 20_000

interface ContextResult {
  id: string
  kind: 'file' | 'memory'
  title: string
  source: string
  content: string
  truncated?: boolean
  size?: number
}

interface FileCandidate {
  path: string
  name: string
}

interface MemoryEntry {
  source: string
  content: string
}

export function createContextRoutes() {
  return new Hono()
    .get('/files', async (c) => {
      const cwdParam = c.req.query('cwd')?.trim()
      if (!cwdParam) return c.json({ error: 'cwd is required' }, 400)

      const query = c.req.query('q')?.trim() ?? ''
      const cwd = resolve(cwdParam)
      try {
        const cwdStat = await stat(cwd)
        if (!cwdStat.isDirectory()) {
          return c.json({ error: 'cwd must be a directory' }, 400)
        }
      } catch {
        return c.json({ error: 'cwd is not readable' }, 400)
      }

      const candidates: FileCandidate[] = []
      for await (const path of walkFiles(cwd, cwd)) {
        if (isBinaryPath(path)) continue
        candidates.push({ path, name: basename(path) })
        if (candidates.length >= MAX_FILE_SCAN) break
      }

      const ranked = rankFileCandidates(candidates, query).slice(0, MAX_RESULTS)
      const files: ContextResult[] = []
      for (const candidate of ranked) {
        const file = await readContextFile(cwd, candidate)
        if (file) files.push(file)
      }

      return c.json({ files })
    })
    .get('/memories', async (c) => {
      const query = c.req.query('q')?.trim() ?? ''
      const memories = rankMemoryEntries(await loadMemoryEntries(), query)
        .slice(0, MAX_RESULTS)
        .map((entry, index): ContextResult => {
          const title = firstContentLine(entry.content) || entry.source
          const { content, truncated } = truncateContent(entry.content)
          return {
            id: `memory:${entry.source}:${index}:${title}`,
            kind: 'memory',
            title,
            source: entry.source,
            content,
            truncated,
          }
        })

      return c.json({ memories })
    })
}

function rankFileCandidates(
  candidates: FileCandidate[],
  query: string,
): FileCandidate[] {
  if (!query) {
    return [...candidates].sort((a, b) => a.path.localeCompare(b.path))
  }

  const fuse = new Fuse(candidates, {
    keys: [
      { name: 'name', weight: 0.7 },
      { name: 'path', weight: 0.3 },
    ],
    threshold: 0.45,
  })
  return fuse.search(query).map((result) => result.item)
}

async function readContextFile(
  cwd: string,
  candidate: FileCandidate,
): Promise<ContextResult | null> {
  const absolutePath = resolve(cwd, candidate.path)

  try {
    const fileStat = await stat(absolutePath)
    if (!fileStat.isFile() || fileStat.size > MAX_FILE_BYTES) return null

    const raw = await readFile(absolutePath, 'utf-8')
    if (raw.includes('\0')) return null

    const { content, truncated } = truncateContent(raw)
    return {
      id: `file:${candidate.path}`,
      kind: 'file',
      title: candidate.name,
      source: candidate.path,
      content,
      truncated,
      size: fileStat.size,
    }
  } catch {
    return null
  }
}

async function loadMemoryEntries(): Promise<MemoryEntry[]> {
  let files: string[]
  try {
    files = (await readdir(getMemoryDir()))
      .filter((file) => file.endsWith('.md'))
      .sort()
      .reverse()
  } catch {
    return []
  }

  const entries: MemoryEntry[] = []
  for (const file of files) {
    try {
      const content = await readFile(resolve(getMemoryDir(), file), 'utf-8')

      const sections = content.split(/^## /m).filter(Boolean)
      for (const section of sections) {
        entries.push({ source: file, content: `## ${section}`.trim() })
      }

      for (const line of content.split('\n')) {
        const trimmed = line.trim()
        if (trimmed && !trimmed.startsWith('#')) {
          entries.push({ source: file, content: trimmed })
        }
      }
    } catch {
      // Skip unreadable memory files.
    }
  }
  return dedupeMemoryEntries(entries)
}

function rankMemoryEntries(
  entries: MemoryEntry[],
  query: string,
): MemoryEntry[] {
  if (!query) return entries

  const fuse = new Fuse(entries, {
    keys: ['content', 'source'],
    threshold: 0.4,
  })
  return fuse.search(query).map((result) => result.item)
}

function dedupeMemoryEntries(entries: MemoryEntry[]): MemoryEntry[] {
  const seen = new Set<string>()
  return entries.filter((entry) => {
    const key = `${entry.source}\n${entry.content}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

function truncateContent(raw: string): { content: string; truncated: boolean } {
  if (raw.length <= MAX_CONTENT_CHARS) {
    return { content: raw, truncated: false }
  }
  return {
    content: raw.slice(0, MAX_CONTENT_CHARS),
    truncated: true,
  }
}

function firstContentLine(content: string): string {
  const line = content
    .split('\n')
    .map((each) => each.replace(/^#+\s*/, '').trim())
    .find(Boolean)
  if (!line) return ''
  return line.length > 80 ? `${line.slice(0, 77)}...` : line
}
