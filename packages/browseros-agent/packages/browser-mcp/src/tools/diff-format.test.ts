import { describe, expect, it } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { SnapshotDiff } from '@browseros/browser-core/core/snapshot/diff'
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
