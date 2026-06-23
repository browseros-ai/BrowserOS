import { describe, expect, it } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { z } from 'zod'
import type { BrowserSession } from '../../../src/browser/core/session'
import {
  defineTool,
  errorResult,
  executeTool,
} from '../../../src/tools/browser/framework'
import { registerBrowserTools } from '../../../src/tools/browser/register'

type RegisteredHandler = (args: Record<string, unknown>) => Promise<{
  content: unknown
  isError?: boolean
  structuredContent?: unknown
}>

function createFakeServer() {
  const handlers = new Map<string, RegisteredHandler>()

  return {
    handlers,
    server: {
      registerTool(
        name: string,
        _config: { description: string; inputSchema?: unknown },
        handler: RegisteredHandler,
      ) {
        handlers.set(name, handler)
      },
    },
  }
}

function textOf(result: { content?: unknown }): string {
  if (!Array.isArray(result.content)) return ''
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
  const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-framework-test-'))
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

async function withBrowserosFile<T>(run: () => Promise<T>): Promise<T> {
  const previous = process.env.BROWSEROS_DIR
  const parentDir = mkdtempSync(join(tmpdir(), 'browseros-framework-test-'))
  const browserosPath = join(parentDir, 'not-a-directory')
  writeFileSync(browserosPath, 'not a directory')
  process.env.BROWSEROS_DIR = browserosPath
  try {
    return await run()
  } finally {
    if (previous === undefined) {
      delete process.env.BROWSEROS_DIR
    } else {
      process.env.BROWSEROS_DIR = previous
    }
    rmSync(parentDir, { recursive: true, force: true })
  }
}

describe('browser tool framework post-actions', () => {
  it('runs compact ToolResponse post-actions after the handler', async () => {
    const events: string[] = []
    const postActionTool = defineTool({
      name: 'post_action_test',
      description: 'Test post-action execution.',
      input: z.object({ page: z.number().int() }),
      handler: async (args, _ctx, response) => {
        events.push('handler')
        response.text('handler output')
        response.data({ page: args.page, url: 'https://example.com/post' })
        response.includeSnapshot(args.page)
      },
    })
    const session = {
      observe: (page: number) => ({
        snapshot: async () => {
          events.push(`snapshot:${page}`)
          return { text: '- button "Submit" [ref=e1]' }
        },
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com/post' }),
        getTabId: () => 1234,
      },
    } as unknown as BrowserSession

    const result = await executeTool(postActionTool, { page: 7 }, { session })
    const text = textOf(result)

    expect(events).toEqual(['handler', 'snapshot:7'])
    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({
      page: 7,
      url: 'https://example.com/post',
    })
    expect(result.metadata).toEqual({ tabId: 1234 })
    expect(text.indexOf('handler output')).toBeLessThan(
      text.indexOf('--- Additional context'),
    )
    expect(text).toContain('[Page 7 snapshot]')
    expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
    expect(text).toContain('- button "Submit" [ref=e1]')
    expect(text).toContain('[END_UNTRUSTED_PAGE_CONTENT')
  })

  it('writes large snapshot post-actions to a BrowserOS output file', async () => {
    await withBrowserosDir(async () => {
      const firstMarker = 'first-node'
      const lastMarker = 'last-node'
      const largeSnapshot = `${firstMarker}\n${'x '.repeat(23_000)}${lastMarker}`
      const postActionTool = defineTool({
        name: 'large_snapshot_post_action_test',
        description: 'Test large snapshot post-action execution.',
        input: z.object({ page: z.number().int() }),
        handler: async (args, _ctx, response) => {
          response.includeSnapshot(args.page)
        },
      })
      const session = {
        observe: () => ({
          snapshot: async () => ({ text: largeSnapshot }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large-snapshot' }),
          getTabId: () => undefined,
        },
      } as unknown as BrowserSession

      const result = await executeTool(postActionTool, { page: 8 }, { session })
      const text = textOf(result)
      const savedPath = text.match(/saved to: (.+\.md)/)?.[1]

      expect(result.isError).toBeFalsy()
      expect(text).toContain('[Page 8 snapshot]')
      expect(text).toContain('estimated tokens')
      expect(savedPath).toBeTruthy()
      expect(text).toContain('Showing the first 5000 estimated tokens inline')
      expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(text).toContain(firstMarker)
      expect(text).not.toContain(lastMarker)
      expect(readFileSync(savedPath ?? '', 'utf8')).toContain(lastMarker)
    })
  })

  it('keeps large snapshot post-actions visible when output file writes fail', async () => {
    await withBrowserosFile(async () => {
      const firstMarker = 'first-node'
      const lastMarker = 'last-node'
      const largeSnapshot = `${firstMarker}\n${'x '.repeat(23_000)}${lastMarker}`
      const postActionTool = defineTool({
        name: 'large_snapshot_post_action_failure_test',
        description: 'Test failed large snapshot post-action output.',
        input: z.object({ page: z.number().int() }),
        handler: async (args, _ctx, response) => {
          response.includeSnapshot(args.page)
        },
      })
      const session = {
        observe: () => ({
          snapshot: async () => ({ text: largeSnapshot }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large-snapshot' }),
          getTabId: () => undefined,
        },
      } as unknown as BrowserSession

      const result = await executeTool(postActionTool, { page: 8 }, { session })
      const text = textOf(result)

      expect(result.isError).toBeFalsy()
      expect(text).toContain('[Page 8 snapshot]')
      expect(text).toContain('could not be saved to a BrowserOS output file')
      expect(text).toContain('Showing the first')
      expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(text).toContain(firstMarker)
      expect(text).toContain('x x x')
      expect(text).not.toContain(lastMarker)
    })
  })

  it('runs diff post-actions through ToolResponse', async () => {
    const events: string[] = []
    const postActionTool = defineTool({
      name: 'diff_post_action_test',
      description: 'Test diff post-action execution.',
      input: z.object({ page: z.number().int() }),
      handler: async (args, _ctx, response) => {
        events.push('handler')
        response.text('handler output')
        response.includeDiff(args.page)
      },
    })
    const session = {
      observe: (page: number) => ({
        diff: async () => {
          events.push(`diff:${page}`)
          return {
            changed: true,
            text: '+   button "Saved" [ref=e1]\n1 added, 0 removed',
            added: 1,
            removed: 0,
            afterUrl: 'https://example.com/current',
          }
        },
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com/stale' }),
        getTabId: () => undefined,
      },
    } as unknown as BrowserSession

    const result = await executeTool(postActionTool, { page: 3 }, { session })
    const text = textOf(result)

    expect(events).toEqual(['handler', 'diff:3'])
    expect(result.isError).toBeFalsy()
    expect(text.indexOf('handler output')).toBeLessThan(
      text.indexOf('--- Additional context'),
    )
    expect(text).toContain('[Page 3 diff]')
    expect(text).toContain('origin=https://example.com/current')
    expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
    expect(text).toContain('+   button "Saved" [ref=e1]')
    expect(text).not.toContain('origin=https://example.com/stale')
  })

  it('writes large diff post-actions to a BrowserOS output file', async () => {
    await withBrowserosDir(async () => {
      const firstMarker = 'first-diff-node'
      const lastMarker = 'last-diff-node'
      const largeDiff = `${firstMarker}\n${'x'.repeat(30_001)}\n${lastMarker}`
      const postActionTool = defineTool({
        name: 'large_diff_post_action_test',
        description: 'Test large diff post-action execution.',
        input: z.object({ page: z.number().int() }),
        handler: async (args, _ctx, response) => {
          response.includeDiff(args.page)
        },
      })
      const session = {
        observe: () => ({
          diff: async () => ({
            changed: true,
            text: largeDiff,
            added: 1,
            removed: 0,
            afterUrl: 'https://example.com/large',
          }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large' }),
          getTabId: () => undefined,
        },
      } as unknown as BrowserSession

      const result = await executeTool(postActionTool, { page: 4 }, { session })
      const text = textOf(result)
      const savedPath = text.match(/saved to: (.+\.md)/)?.[1]

      expect(result.isError).toBeFalsy()
      expect(text).toContain('[Page 4 diff]')
      expect(text).toContain('estimated tokens')
      expect(savedPath).toBeTruthy()
      expect(text).toContain('Showing the first 5000 estimated tokens inline')
      expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(text).toContain(firstMarker)
      expect(text).not.toContain(lastMarker)
      expect(readFileSync(savedPath ?? '', 'utf8')).toContain(lastMarker)
    })
  })

  it('keeps large diff post-actions visible when output file writes fail', async () => {
    await withBrowserosFile(async () => {
      const firstMarker = 'first-diff-node'
      const lastMarker = 'last-diff-node'
      const largeDiff = `${firstMarker}\n${'x'.repeat(30_001)}\n${lastMarker}`
      const postActionTool = defineTool({
        name: 'large_diff_post_action_failure_test',
        description: 'Test failed large diff post-action output.',
        input: z.object({ page: z.number().int() }),
        handler: async (args, _ctx, response) => {
          response.includeDiff(args.page)
        },
      })
      const session = {
        observe: () => ({
          diff: async () => ({
            changed: true,
            text: largeDiff,
            added: 1,
            removed: 0,
            afterUrl: 'https://example.com/large',
          }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large' }),
          getTabId: () => undefined,
        },
      } as unknown as BrowserSession

      const result = await executeTool(postActionTool, { page: 4 }, { session })
      const text = textOf(result)

      expect(result.isError).toBeFalsy()
      expect(text).toContain('[Page 4 diff]')
      expect(text).toContain('saving it to a BrowserOS output file failed')
      expect(text).toContain('Showing the first')
      expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(text).toContain(firstMarker)
      expect(text).toContain('xxx')
      expect(text).not.toContain(lastMarker)
    })
  })

  it('keeps compact error results from running undeclared post-actions', async () => {
    const errorTool = defineTool({
      name: 'error_test',
      description: 'Test error result handling.',
      input: z.object({ page: z.number().int() }),
      handler: async () => errorResult('error_test: nope'),
    })
    const session = {
      observe: () => ({
        snapshot: async () => {
          throw new Error('snapshot should not run')
        },
      }),
      pages: {
        getTabId: () => undefined,
      },
    } as unknown as BrowserSession

    const result = await executeTool(errorTool, { page: 2 }, { session })

    expect(result.isError).toBe(true)
    expect(textOf(result)).toBe('error_test: nope')
  })

  it('attaches navigate snapshots through the shared post-action path', async () => {
    const fake = createFakeServer()
    const calls: string[] = []
    let currentUrl = 'https://example.com/before'
    const session = {
      nav: () => ({
        goto: async (url: string) => calls.push(`goto:${url}`),
      }),
      observe: (page: number) => ({
        snapshot: async () => {
          calls.push(`snapshot:${page}`)
          return { text: '- heading "Arrived" [ref=e1]' }
        },
      }),
      pages: {
        getInfo: () => {
          calls.push('getInfo')
          return { url: currentUrl }
        },
        refresh: async () => {
          calls.push('refresh')
          currentUrl = 'https://example.com/after'
          return { url: currentUrl }
        },
        getTabId: () => undefined,
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('navigate')?.({
      page: 9,
      action: 'url',
      url: 'https://example.com/before',
    })
    const text = textOf(result ?? {})

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({
      page: 9,
      url: 'https://example.com/after',
    })
    expect(calls).toEqual([
      'goto:https://example.com/before',
      'refresh',
      'snapshot:9',
      'getInfo',
    ])
    expect(text).toContain('navigated (url) -> https://example.com/after')
    expect(text).toContain('origin=https://example.com/after')
    expect(text.indexOf('navigated (url)')).toBeLessThan(
      text.indexOf('--- Additional context'),
    )
    expect(text).toContain('[Page 9 snapshot]')
    expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
    expect(text).toContain('- heading "Arrived" [ref=e1]')
  })

  it('does not snapshot navigate validation errors', async () => {
    const fake = createFakeServer()
    const calls: string[] = []
    const session = {
      nav: () => ({
        goto: async () => calls.push('goto'),
      }),
      observe: () => ({
        snapshot: async () => calls.push('snapshot'),
      }),
      pages: {
        getTabId: () => undefined,
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('navigate')?.({
      page: 9,
      action: 'url',
    })

    expect(result?.isError).toBe(true)
    expect(result?.content).toEqual([
      { type: 'text', text: 'navigate: url is required for action="url".' },
    ])
    expect(calls).toEqual([])
  })
})
