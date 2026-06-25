import { describe, it } from 'bun:test'
import assert from 'node:assert'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { ToolResponse } from '@browseros/browser-mcp/response'

function textOf(result: {
  content: { type: string; text?: string }[]
}): string {
  return result.content
    .filter((item) => item.type === 'text')
    .map((item) => item.text)
    .join('\n')
}

async function withBrowserosDir<T>(run: () => Promise<T>): Promise<T> {
  const previous = process.env.BROWSEROS_DIR
  const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-response-test-'))
  process.env.BROWSEROS_DIR = browserosDir
  try {
    return await run()
  } finally {
    if (previous === undefined) {
      delete process.env.BROWSEROS_DIR
    } else {
      process.env.BROWSEROS_DIR = previous
    }
    rmSync(browserosDir, { recursive: true, force: true })
  }
}

function createSession(overrides: {
  observe?: unknown
  pages?: unknown
}): BrowserSession {
  return {
    observe: overrides.observe,
    pages: overrides.pages,
  } as unknown as BrowserSession
}

describe('ToolResponse', () => {
  it('accumulates structured content from data()', () => {
    const response = new ToolResponse()
    response.data('action', 'click')
    response.data({ page: 1, element: 42 })

    const result = response.toResult()
    assert.deepStrictEqual(result.structuredContent, {
      action: 'click',
      page: 1,
      element: 42,
    })
  })

  it('overwrites keys on repeated data() writes', () => {
    const response = new ToolResponse()
    response.data('count', 1)
    response.data({ count: 2 })
    response.data('count', 3)

    const result = response.toResult()
    assert.deepStrictEqual(result.structuredContent, { count: 3 })
  })

  it('times out slow post-actions without failing tool output', async () => {
    const response = new ToolResponse({ postActionTimeoutMs: 25 })
    response.text('ok')
    response.includeSnapshot(1)

    const session = createSession({
      observe: () => ({
        snapshot: async () => await new Promise(() => {}),
      }),
    })

    const start = Date.now()
    const result = await response.buildForSession(session)
    const elapsed = Date.now() - start

    assert.ok(elapsed < 250, `Expected fast timeout, got ${elapsed}ms`)
    assert.ok(!result.isError)

    const text = textOf(result)
    assert.ok(text.includes('ok'))
    assert.ok(!text.includes('[Page 1 snapshot]'))
  })

  it('includes snapshot output when post-action completes in time', async () => {
    const response = new ToolResponse({ postActionTimeoutMs: 200 })
    response.text('ok')
    response.includeSnapshot(1)

    const session = createSession({
      observe: () => ({
        snapshot: async () => ({ text: '[42] button "Submit"', refs: {} }),
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com/small' }),
      },
    })

    const result = await response.buildForSession(session)
    const text = textOf(result)

    assert.ok(text.includes('ok'))
    assert.ok(text.includes('[Page 1 snapshot]'))
    assert.ok(text.includes('[42] button "Submit"'))
  })

  it('writes large snapshot post-actions to a BrowserOS output file', async () => {
    await withBrowserosDir(async () => {
      const response = new ToolResponse({ postActionTimeoutMs: 200 })
      const firstMarker = 'first-node'
      const lastMarker = 'last-node'
      const largeSnapshot = `${firstMarker}\n${'x '.repeat(23_000)}${lastMarker}`
      response.text('ok')
      response.includeSnapshot(1)

      const session = createSession({
        observe: () => ({
          snapshot: async () => ({ text: largeSnapshot, refs: {} }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large' }),
        },
      })

      const result = await response.buildForSession(session)
      const text = textOf(result)
      const savedPath = text.match(/saved to: (.+\.md)/)?.[1]

      assert.ok(!result.isError)
      assert.ok(text.includes('ok'))
      assert.ok(text.includes('[Page 1 snapshot]'))
      assert.ok(text.includes('Large snapshot ('))
      assert.ok(text.includes('estimated tokens'))
      assert.ok(savedPath)
      assert.ok(text.includes('Showing the first 5000 estimated tokens inline'))
      assert.ok(text.includes('[UNTRUSTED_PAGE_CONTENT'))
      assert.ok(text.includes(firstMarker))
      assert.ok(!text.includes(lastMarker))
      assert.ok(readFileSync(savedPath ?? '', 'utf8').includes(lastMarker))
    })
  })

  it('includes diff output when buildForSession receives a diff post-action', async () => {
    const response = new ToolResponse({ postActionTimeoutMs: 200 })
    response.text('ok')
    response.includeDiff(1)

    const session = createSession({
      observe: () => ({
        diff: async () => ({
          changed: true,
          text: '+   button "Saved" [ref=e1]\n1 added, 0 removed',
          added: 1,
          removed: 0,
          afterUrl: 'https://example.com/current',
        }),
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com/stale' }),
      },
    })

    const result = await response.buildForSession(session)
    const text = textOf(result)

    assert.ok(text.includes('ok'))
    assert.ok(text.includes('[Page 1 diff]'))
    assert.ok(text.includes('origin=https://example.com/current'))
    assert.ok(text.includes('+   button "Saved" [ref=e1]'))
    assert.ok(!text.includes('origin=https://example.com/stale'))
  })
})
