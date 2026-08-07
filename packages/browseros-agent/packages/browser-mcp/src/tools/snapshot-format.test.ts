import { describe, expect, it } from 'bun:test'
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { formatSnapshotResult } from './snapshot-format'

async function withBrowserosDir<T>(run: () => Promise<T>): Promise<T> {
  const previous = process.env.BROWSEROS_DIR
  const browserosDir = mkdtempSync(join(tmpdir(), 'snapshot-format-test-'))
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
  const browserosDir = mkdtempSync(join(tmpdir(), 'snapshot-format-fail-'))
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

describe('formatSnapshotResult', () => {
  it('returns small snapshots inline with metadata-only structured content', async () => {
    const result = await formatSnapshotResult(
      '- button "Save" [ref=e1]',
      'https://example.com/small',
    )

    expect(result.text).toContain('[UNTRUSTED_PAGE_CONTENT')
    expect(result.text).toContain('- button "Save" [ref=e1]')
    expect(result.structured).toEqual({
      contentLength: result.text.length,
      tokenEstimate: expect.any(Number),
      writtenToFile: false,
    })
    expect(result.structured).not.toHaveProperty('snapshot')
  })

  it('writes large snapshots to a file without duplicating the snapshot in structured content', async () => {
    await withBrowserosDir(async () => {
      const firstMarker = 'first-node'
      const lastMarker = 'last-node'
      const result = await formatSnapshotResult(
        `${firstMarker}\n${'x '.repeat(23_000)}${lastMarker}`,
        'https://example.com/large',
      )
      const data = result.structured as
        | {
            contentLength: number
            tokenEstimate: number
            writtenToFile: boolean
            path: string
          }
        | undefined

      expect(data).toMatchObject({
        writtenToFile: true,
      })
      const path = data?.path
      expect(typeof path).toBe('string')
      if (typeof path !== 'string') throw new Error('expected output path')
      expect(typeof data?.tokenEstimate).toBe('number')
      expect(data?.tokenEstimate).toBeGreaterThan(15_000)
      expect(data).not.toHaveProperty('snapshot')
      expect(result.text).toContain(path)
      expect(result.text).toContain(
        'Showing the first 5000 estimated tokens inline',
      )
      expect(result.text).toContain(firstMarker)
      expect(result.text).not.toContain(lastMarker)
      expect(existsSync(path)).toBe(true)
      const savedContent = readFileSync(path, 'utf8')
      expect(savedContent).toContain(lastMarker)
      expect(data?.contentLength).toBe(savedContent.length)
    })
  })

  it('keeps save-failure structured content metadata-only', async () => {
    await withOutputWriteFailure(async () => {
      const lastMarker = 'last-node'
      const result = await formatSnapshotResult(
        `first-node\n${'x '.repeat(23_000)}${lastMarker}`,
        'https://example.com/fail',
      )

      expect(result.text).toContain('could not be saved')
      expect(result.text).not.toContain(lastMarker)
      expect(result.structured).toMatchObject({
        writtenToFile: false,
        outputWriteFailed: true,
        error: expect.any(String),
      })
      expect(result.structured).not.toHaveProperty('snapshot')
      expect(JSON.stringify(result.structured)).not.toContain(lastMarker)
    })
  })
})
