import { describe, expect, it } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import { executeTool } from './framework'
import { grep } from './grep'

function sessionWithSnapshot(text: string): BrowserSession {
  return {
    observe: () => ({ snapshot: async () => ({ text }) }),
    pages: { getInfo: () => ({ url: 'https://example.com' }) },
  } as unknown as BrowserSession
}

function textOf(result: { content?: unknown } | undefined): string {
  if (!Array.isArray(result?.content)) return ''
  return result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        typeof item === 'object' &&
        item !== null &&
        'type' in item &&
        item.type === 'text' &&
        'text' in item &&
        typeof item.text === 'string',
    )
    .map((item) => item.text)
    .join('\n')
}

async function withBrowserosDir<T>(run: () => Promise<T>): Promise<T> {
  const previous = process.env.BROWSEROS_DIR
  const browserosDir = mkdtempSync(join(tmpdir(), 'grep-test-'))
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
  const browserosDir = mkdtempSync(join(tmpdir(), 'grep-fail-'))
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

describe('grep tool', () => {
  it('returns small matches in bounded structured output', async () => {
    const result = await executeTool(
      grep,
      { page: 4, pattern: 'save', over: 'ax' },
      {
        session: sessionWithSnapshot(
          'button "Save" [ref=e1]\nlink "Home"\nbutton "Save draft" [ref=e2]',
        ),
      },
    )

    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({
      page: 4,
      over: 'ax',
      count: 2,
      matches: ['button "Save" [ref=e1]', 'button "Save draft" [ref=e2]'],
    })
    expect(textOf(result)).toContain('button "Save" [ref=e1]')
  })

  it('reports zero matches with an empty matches array', async () => {
    const result = await executeTool(
      grep,
      { page: 4, pattern: 'checkout', over: 'ax' },
      { session: sessionWithSnapshot('link "Home"\nlink "About"') },
    )

    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({
      page: 4,
      over: 'ax',
      count: 0,
      matches: [],
    })
  })

  it('preserves refs while clamping matches and spills the full line', async () => {
    await withBrowserosDir(async () => {
      const ref = '[ref=e123]'
      const line = `needle ${'x'.repeat(100_000)} ${ref}`
      const result = await executeTool(
        grep,
        { page: 4, pattern: 'needle', over: 'ax' },
        { session: sessionWithSnapshot(line) },
      )
      const data = result.structuredContent as
        | {
            page: number
            over: string
            count: number
            matches: string[]
            contentLength: number
            writtenToFile: boolean
            path: string
          }
        | undefined
      const text = textOf(result)
      const renderedLine = text.split('\n')[1] ?? ''

      expect(result.isError).toBeFalsy()
      expect(data).toMatchObject({
        page: 4,
        over: 'ax',
        count: 1,
        matches: [expect.stringMatching(/\.\.\. \[truncated\] \[ref=e123\]$/)],
        contentLength: line.length,
        writtenToFile: true,
      })
      const path = data?.path
      expect(typeof path).toBe('string')
      if (typeof path !== 'string') throw new Error('expected output path')
      expect(renderedLine.length).toBeLessThanOrEqual(
        TOOL_LIMITS.GREP_MATCH_LINE_MAX_CHARS,
      )
      expect(renderedLine).toContain('... [truncated]')
      expect(renderedLine).toEndWith(ref)
      expect(data?.matches).toEqual([renderedLine])
      expect(text).toContain(path)
      expect(readFileSync(path, 'utf8')).toContain(line)
    })
  })

  it('does not preserve ref-like suffixes that exceed the line budget', async () => {
    await withBrowserosDir(async () => {
      const oversizedSuffix = ` [ref=e${'1'.repeat(1_000)}]`
      const line = `needle ${'x'.repeat(6_000)}${oversizedSuffix}`
      const result = await executeTool(
        grep,
        { page: 4, pattern: 'needle', over: 'ax' },
        { session: sessionWithSnapshot(line) },
      )
      const data = result.structuredContent as { matches: string[] } | undefined
      const renderedLine = data?.matches[0] ?? ''

      expect(result.isError).toBeFalsy()
      expect(renderedLine.length).toBeLessThanOrEqual(
        TOOL_LIMITS.GREP_MATCH_LINE_MAX_CHARS,
      )
      expect(renderedLine).toEndWith('... [truncated]')
      expect(renderedLine).not.toContain(oversizedSuffix)
    })
  })

  it('keeps clamped matches inline when spilling fails', async () => {
    await withOutputWriteFailure(async () => {
      const tail = 'tail-marker'
      const line = `needle ${'x'.repeat(100_000)} ${tail}`
      const result = await executeTool(
        grep,
        { page: 4, pattern: 'needle', over: 'ax' },
        { session: sessionWithSnapshot(line) },
      )
      const data = result.structuredContent as
        | {
            page: number
            over: string
            count: number
            matches: string[]
            contentLength: number
            writtenToFile: boolean
            outputWriteFailed: boolean
            error: string
          }
        | undefined
      const text = textOf(result)
      const renderedLine = text.split('\n')[1] ?? ''

      expect(result.isError).toBeFalsy()
      expect(data).toMatchObject({
        page: 4,
        over: 'ax',
        count: 1,
        matches: [expect.stringContaining('... [truncated]')],
        contentLength: line.length,
        writtenToFile: false,
        outputWriteFailed: true,
        error: expect.any(String),
      })
      expect(data).not.toHaveProperty('path')
      expect(renderedLine.length).toBeLessThanOrEqual(
        TOOL_LIMITS.GREP_MATCH_LINE_MAX_CHARS,
      )
      expect(renderedLine).toContain('... [truncated]')
      expect(data?.matches).toEqual([renderedLine])
      expect(text).toContain('could not be saved')
      expect(text).not.toContain(tail)
    })
  })

  it('clamps requested limits to the shared maximum', async () => {
    const haystack = Array.from(
      { length: TOOL_LIMITS.GREP_MAX_MATCHES + 5 },
      (_, index) => `match ${index}`,
    ).join('\n')
    const result = await executeTool(
      grep,
      {
        page: 4,
        pattern: 'match',
        over: 'ax',
        limit: TOOL_LIMITS.GREP_MAX_MATCHES + 100,
      },
      { session: sessionWithSnapshot(haystack) },
    )

    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({
      page: 4,
      over: 'ax',
      count: TOOL_LIMITS.GREP_MAX_MATCHES,
      matches: Array.from(
        { length: TOOL_LIMITS.GREP_MAX_MATCHES },
        (_, index) => `match ${index}`,
      ),
    })
    expect(textOf(result)).toContain(
      `match ${TOOL_LIMITS.GREP_MAX_MATCHES - 1}`,
    )
    expect(textOf(result)).not.toContain(
      `match ${TOOL_LIMITS.GREP_MAX_MATCHES}`,
    )
  })
})
