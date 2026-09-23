import { describe, expect, it } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  diffSnapshots,
  type SnapshotDiff,
} from '@browseros/browser-core/core/snapshot/diff'
import { formatDiffResult } from './diff-format'

async function withBrowserosDir<T>(run: () => Promise<T>): Promise<T> {
  const previous = process.env.BROWSEROS_DIR
  const browserosDir = mkdtempSync(join(tmpdir(), 'diff-format-test-'))
  process.env.BROWSEROS_DIR = browserosDir
  try {
    return await run()
  } finally {
    restoreBrowserosDir(previous)
    rmSync(browserosDir, { recursive: true, force: true })
  }
}

async function withOutputWriteFailure<T>(run: () => Promise<T>): Promise<T> {
  const previous = process.env.BROWSEROS_DIR
  const browserosDir = mkdtempSync(join(tmpdir(), 'diff-format-fail-'))
  const filePath = join(browserosDir, 'not-a-directory')
  writeFileSync(filePath, 'x')
  process.env.BROWSEROS_DIR = filePath
  try {
    return await run()
  } finally {
    restoreBrowserosDir(previous)
    rmSync(browserosDir, { recursive: true, force: true })
  }
}

function restoreBrowserosDir(previous: string | undefined): void {
  if (previous === undefined) {
    delete process.env.BROWSEROS_DIR
  } else {
    process.env.BROWSEROS_DIR = previous
  }
}

function changedDiff(
  text: string,
  extra: Partial<SnapshotDiff> = {},
): SnapshotDiff {
  return {
    changed: true,
    text,
    added: 1,
    removed: 2,
    ...extra,
  }
}

describe('formatDiffResult', () => {
  it('preserves small diff semantics without duplicating diff text', async () => {
    const result = await formatDiffResult(
      changedDiff('+ button "Save" [ref=e1]'),
      'https://example.com/current',
    )

    expect(result.text).toContain('+ button "Save" [ref=e1]')
    expect(result.structured).toEqual({
      changed: true,
      added: 1,
      removed: 2,
    })
    expect(result.structured).not.toHaveProperty('diff')
    expect(result.structured).not.toHaveProperty('snapshot')
  })

  it('keeps unchanged diffs compact', async () => {
    const result = await formatDiffResult(
      { changed: false, text: '', added: 0, removed: 0 },
      'https://example.com/current',
    )

    expect(result.text).toBe('no change since last snapshot')
    expect(result.structured).toEqual({ changed: false })
  })

  it('renders over-budget diff guidance inline with bounded metadata', async () => {
    const before = Array.from(
      { length: 2_001 },
      (_, i) => `- before node ${i}`,
    ).join('\n')
    const after = Array.from(
      { length: 2_000 },
      (_, i) => `- after node ${i}`,
    ).join('\n')

    const result = await formatDiffResult(
      diffSnapshots(before, after),
      'https://example.com/dynamic',
    )

    expect(result.text).toContain('Snapshot changed substantially')
    expect(result.text).toContain('Take a fresh snapshot for the current state')
    expect(result.text).not.toContain('UNTRUSTED_PAGE_CONTENT')
    expect(result.structured).toEqual({
      changed: true,
      added: 2_000,
      removed: 2_001,
      lineDiffSkipped: true,
    })
    expect(result.structured).not.toHaveProperty('truncated')
    expect(result.structured).not.toHaveProperty('path')
  })

  it('writes large diffs to a file with metadata-only structured content', async () => {
    await withBrowserosDir(async () => {
      const firstMarker = 'first-diff-node'
      const lastMarker = 'last-diff-node'
      const result = await formatDiffResult(
        changedDiff(`${firstMarker}\n${'x'.repeat(30_001)}\n${lastMarker}`),
        'https://example.com/large',
      )
      const data = result.structured as
        | {
            changed: boolean
            added: number
            removed: number
            truncated: boolean
            tokenEstimate: number
            writtenToFile: boolean
            path: string
            contentLength: number
          }
        | undefined

      expect(data).toMatchObject({
        changed: true,
        added: 1,
        removed: 2,
        truncated: true,
        writtenToFile: true,
      })
      const path = data?.path
      expect(typeof path).toBe('string')
      if (typeof path !== 'string') throw new Error('expected output path')
      expect(typeof data?.tokenEstimate).toBe('number')
      expect(data?.tokenEstimate).toBeGreaterThan(10_000)
      expect(data).not.toHaveProperty('diff')
      expect(data).not.toHaveProperty('snapshot')
      expect(result.text).toContain(path)
      expect(result.text).toContain(firstMarker)
      expect(result.text).not.toContain(lastMarker)
      const savedContent = readFileSync(path, 'utf8')
      expect(savedContent).toContain(lastMarker)
      expect(data?.contentLength).toBe(savedContent.length)
    })
  })

  it('preserves URL-change fields without structured snapshot text', async () => {
    const result = await formatDiffResult(
      changedDiff('- heading "Destination"', {
        added: 0,
        removed: 0,
        urlChanged: true,
        beforeUrl: 'https://example.com/start',
        afterUrl: 'https://example.com/destination',
      }),
      'https://example.com/destination',
    )

    expect(result.text).toContain('URL changed')
    expect(result.text).toContain('- heading "Destination"')
    expect(result.structured).toEqual({
      changed: true,
      added: 0,
      removed: 0,
      urlChanged: true,
      beforeUrl: 'https://example.com/start',
      afterUrl: 'https://example.com/destination',
    })
    expect(result.structured).not.toHaveProperty('snapshot')
    expect(result.structured).not.toHaveProperty('diff')
  })

  it('keeps save-failure structured content metadata-only', async () => {
    await withOutputWriteFailure(async () => {
      const lastMarker = 'last-diff-node'
      const result = await formatDiffResult(
        changedDiff(`first-diff-node\n${'x'.repeat(30_001)}\n${lastMarker}`),
        'https://example.com/fail',
      )

      expect(result.text).toContain(
        'saving it to a BrowserOS output file failed',
      )
      expect(result.text).not.toContain(lastMarker)
      expect(result.structured).toMatchObject({
        changed: true,
        added: 1,
        removed: 2,
        truncated: true,
        writtenToFile: false,
        outputWriteFailed: true,
        error: expect.any(String),
      })
      expect(result.structured).not.toHaveProperty('diff')
      expect(result.structured).not.toHaveProperty('snapshot')
      expect(JSON.stringify(result.structured)).not.toContain(lastMarker)
    })
  })
})

describe('formatDiffResult detail modes (#2700)', () => {
  it('summary returns only change counts, not the diff body', async () => {
    const result = await formatDiffResult(
      changedDiff('+ button "Save" [ref=e1]', { added: 4, removed: 1 }),
      'https://example.com/current',
      'summary',
    )

    expect(result.text).toContain('4 added, 1 removed')
    expect(result.text).not.toContain('+ button "Save" [ref=e1]')
    expect(result.structured).toMatchObject({
      changed: true,
      added: 4,
      removed: 1,
    })
  })

  it('summary on a URL change reports the before and after urls', async () => {
    const result = await formatDiffResult(
      changedDiff('- heading "Home"', {
        added: 0,
        removed: 0,
        urlChanged: true,
        beforeUrl: 'https://example.com/a',
        afterUrl: 'https://example.com/b',
      }),
      'https://example.com/b',
      'summary',
    )

    expect(result.text).toContain('URL changed')
    expect(result.text).toContain('https://example.com/a')
    expect(result.text).toContain('https://example.com/b')
    expect(result.text).not.toContain('- heading "Home"')
  })

  it('maxChars returns the diff whole when it fits the budget', async () => {
    const body = '+ node "Save" [ref=e1]'
    const result = await formatDiffResult(
      changedDiff(body),
      'https://example.com/current',
      { maxChars: 10_000 },
    )

    expect(result.text).toContain(body)
    expect(result.structured).not.toHaveProperty('truncated')
  })

  it('maxChars preserves the URL-change notice on a navigation', async () => {
    const result = await formatDiffResult(
      changedDiff('new page snapshot body', {
        added: 0,
        removed: 0,
        urlChanged: true,
        beforeUrl: 'https://example.com/a',
        afterUrl: 'https://example.com/b',
      }),
      'https://example.com/b',
      { maxChars: 10_000 },
    )

    // A navigation snapshot capped by maxChars must not read as an ordinary diff.
    expect(result.text).toContain('URL changed')
    expect(result.text).toContain('https://example.com/a')
    expect(result.text).toContain('https://example.com/b')
    expect(result.text).toContain('new page snapshot body')
  })

  it('maxChars truncates a large diff inline and spills the rest to a file', async () => {
    await withBrowserosDir(async () => {
      const firstMarker = 'first-diff-node'
      const lastMarker = 'last-diff-node'
      const result = await formatDiffResult(
        changedDiff(`${firstMarker}\n${'x'.repeat(5_000)}\n${lastMarker}`),
        'https://example.com/large',
        { maxChars: 200 },
      )
      const data = result.structured as
        | {
            truncated?: boolean
            writtenToFile?: boolean
            path?: string
            contentLength?: number
          }
        | undefined

      expect(data).toMatchObject({ truncated: true, writtenToFile: true })
      const path = data?.path
      if (typeof path !== 'string') throw new Error('expected output path')
      expect(result.text).toContain('truncated at 200 chars')
      expect(result.text).toContain(firstMarker)
      expect(result.text).not.toContain(lastMarker)
      const saved = readFileSync(path, 'utf8')
      expect(saved).toContain(lastMarker)
    })
  })
})
