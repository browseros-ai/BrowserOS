import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

interface MemoryEntry {
  source: string
  content: string
}

async function loadMemoryEntriesSequential(memoryDir: string): Promise<MemoryEntry[]> {
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

async function loadMemoryEntriesParallel(memoryDir: string): Promise<MemoryEntry[]> {
  let files: string[]
  try {
    files = await readdir(memoryDir)
  } catch {
    return []
  }

  const mdFiles = files.filter((f) => f.endsWith('.md'))

  const filePromises = mdFiles.map(async (file) => {
    try {
      const content = await readFile(join(memoryDir, file), 'utf-8')
      const fileEntries: MemoryEntry[] = []

      // Section-level entries (## delimited blocks)
      const sections = content.split(/^## /m).filter(Boolean)
      for (const section of sections) {
        fileEntries.push({ source: file, content: `## ${section}`.trim() })
      }

      // Line-level entries (individual non-empty lines)
      for (const line of content.split('\n')) {
        const trimmed = line.trim()
        if (trimmed && !trimmed.startsWith('#')) {
          fileEntries.push({ source: file, content: trimmed })
        }
      }
      return fileEntries
    } catch {
      return []
    }
  })

  const results = await Promise.all(filePromises)
  return results.flat()
}

async function run() {
  const memoryDir = '/tmp/test_memory_dir'
  const fs = await import('fs/promises')
  await fs.mkdir(memoryDir, { recursive: true })

  // Create 100 dummy files
  for (let i = 0; i < 100; i++) {
    await fs.writeFile(join(memoryDir, `file_${i}.md`), `## Section 1\nLine 1\nLine 2\n## Section 2\nLine 3\nLine 4\n`)
  }

  // Warmup
  await loadMemoryEntriesSequential(memoryDir)
  await loadMemoryEntriesParallel(memoryDir)

  const numIterations = 100

  const startSeq = performance.now()
  for (let i = 0; i < numIterations; i++) {
    await loadMemoryEntriesSequential(memoryDir)
  }
  const endSeq = performance.now()

  const startPar = performance.now()
  for (let i = 0; i < numIterations; i++) {
    await loadMemoryEntriesParallel(memoryDir)
  }
  const endPar = performance.now()

  console.log(`Sequential: ${endSeq - startSeq}ms`)
  console.log(`Parallel: ${endPar - startPar}ms`)

  await fs.rm(memoryDir, { recursive: true, force: true })
}

run()
