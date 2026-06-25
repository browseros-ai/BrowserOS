import { describe, expect, it } from 'bun:test'
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createBrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import { registerBrowserTools } from '@browseros/browser-mcp/register'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import {
  getToolOutputDir,
  TOOL_OUTPUT_DIR_MODE,
  TOOL_OUTPUT_FILE_MODE,
} from '@browseros/browser-mcp/tool-output-dir'
import {
  defineTool,
  executeTool,
  textResult,
} from '@browseros/browser-mcp/tools/framework'
import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import { z } from 'zod'
import { CHAT_MODE_ALLOWED_TOOLS } from '../../../src/agent/chat-mode'
import { buildBrowserToolSet } from '../../../src/agent/tool-adapter'
import { createReadTool } from '../../../src/tools/filesystem/read'

type RegisteredHandler = (args: Record<string, unknown>) => Promise<{
  content: unknown
  isError?: boolean
  structuredContent?: unknown
}>

function createFakeServer() {
  const handlers = new Map<string, RegisteredHandler>()
  const configs = new Map<
    string,
    { description: string; inputSchema?: unknown }
  >()

  return {
    handlers,
    configs,
    server: {
      registerTool(
        name: string,
        config: { description: string; inputSchema?: unknown },
        handler: RegisteredHandler,
      ) {
        configs.set(name, config)
        handlers.set(name, handler)
      },
    },
  }
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
  const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-output-test-'))
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

async function expectBrowserToolOutputPath(
  filePath: string | undefined,
): Promise<void> {
  expect(filePath).toBeTruthy()
  const path = filePath ?? ''
  const outputDir = await getToolOutputDir()
  expect(realpathSync(dirname(path))).toBe(realpathSync(outputDir))
  if (process.platform !== 'win32') {
    expect(statSync(outputDir).mode & 0o777).toBe(TOOL_OUTPUT_DIR_MODE)
    expect(statSync(path).mode & 0o777).toBe(TOOL_OUTPUT_FILE_MODE)
  }
}

describe('registerBrowserTools', () => {
  it('registers the compact browser tool surface', () => {
    const fake = createFakeServer()
    const session = { pages: {} } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    expect([...fake.handlers.keys()]).toEqual(BROWSER_TOOLS.map((t) => t.name))
    expect(fake.handlers.size).toBe(16)
    expect(fake.configs.get('tabs')?.inputSchema).toBeDefined()
  })

  it('captures JPEG screenshots with default and custom quality', async () => {
    const fake = createFakeServer()
    const captureOptions: unknown[] = []
    const session = {
      screenshot: async (_page: number, options: unknown) => {
        captureOptions.push(options)
        return { data: 'jpeg-data', mimeType: 'image/jpeg', annotations: [] }
      },
      pages: {
        getSession: async () => ({
          session: {
            Page: {
              getLayoutMetrics: async () => ({
                layoutViewport: {
                  pageX: 0,
                  pageY: 0,
                  clientWidth: 2048,
                  clientHeight: 1536,
                },
                cssLayoutViewport: {
                  pageX: 5,
                  pageY: 7,
                  clientWidth: 2048,
                  clientHeight: 1536,
                },
              }),
            },
          },
        }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    await expect(
      fake.handlers.get('screenshot')?.({ page: 1 }),
    ).resolves.toEqual({
      content: [{ type: 'image', data: 'jpeg-data', mimeType: 'image/jpeg' }],
      structuredContent: {
        page: 1,
        format: 'jpeg',
        bytes: Buffer.from('jpeg-data', 'base64').length,
        image: 'jpeg-data',
      },
    })
    await expect(
      fake.handlers.get('screenshot')?.({
        page: 1,
        quality: 60,
        size: { width: 512, height: 384 },
      }),
    ).resolves.toEqual({
      content: [{ type: 'image', data: 'jpeg-data', mimeType: 'image/jpeg' }],
      structuredContent: {
        page: 1,
        format: 'jpeg',
        bytes: Buffer.from('jpeg-data', 'base64').length,
        image: 'jpeg-data',
      },
    })
    expect(captureOptions).toEqual([
      {
        format: 'jpeg',
        quality: 80,
        fullPage: false,
        annotate: true,
        clip: {
          x: 5,
          y: 7,
          width: 2048,
          height: 1536,
          scale: 0.5,
        },
      },
      {
        format: 'jpeg',
        quality: 60,
        fullPage: false,
        annotate: true,
        clip: {
          x: 5,
          y: 7,
          width: 2048,
          height: 1536,
          scale: 0.25,
        },
      },
    ])
  })

  it('omits quality for PNG screenshots and skips size clips for full page', async () => {
    const fake = createFakeServer()
    const captureOptions: unknown[] = []
    let layoutMetricCalls = 0
    const session = {
      screenshot: async (_page: number, options: { format?: string }) => {
        captureOptions.push(options)
        return {
          data: `${options.format}-data`,
          mimeType: `image/${options.format}`,
          annotations: [],
        }
      },
      pages: {
        getSession: async () => ({
          session: {
            Page: {
              getLayoutMetrics: async () => {
                layoutMetricCalls += 1
                return {
                  layoutViewport: {
                    pageX: 0,
                    pageY: 0,
                    clientWidth: 800,
                    clientHeight: 600,
                  },
                  cssLayoutViewport: {
                    pageX: 0,
                    pageY: 0,
                    clientWidth: 800,
                    clientHeight: 600,
                  },
                }
              },
            },
          },
        }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    await expect(
      fake.handlers.get('screenshot')?.({
        page: 1,
        format: 'png',
        quality: 25,
      }),
    ).resolves.toEqual({
      content: [{ type: 'image', data: 'png-data', mimeType: 'image/png' }],
      structuredContent: {
        page: 1,
        format: 'png',
        bytes: Buffer.from('png-data', 'base64').length,
        image: 'png-data',
      },
    })
    await expect(
      fake.handlers.get('screenshot')?.({ page: 1, fullPage: true }),
    ).resolves.toEqual({
      content: [{ type: 'image', data: 'jpeg-data', mimeType: 'image/jpeg' }],
      structuredContent: {
        page: 1,
        format: 'jpeg',
        bytes: Buffer.from('jpeg-data', 'base64').length,
        image: 'jpeg-data',
      },
    })

    expect(layoutMetricCalls).toBe(1)
    expect(captureOptions).toEqual([
      {
        format: 'png',
        fullPage: false,
        annotate: true,
        clip: {
          x: 0,
          y: 0,
          width: 800,
          height: 600,
          scale: 1,
        },
      },
      {
        format: 'jpeg',
        quality: 80,
        fullPage: true,
        annotate: true,
      },
    ])
  })

  it('manages windows through the compact windows tool', async () => {
    const fake = createFakeServer()
    const calls: Array<{ method: string; args?: unknown }> = []
    const window = {
      windowId: 7,
      windowType: 'normal' as const,
      bounds: {},
      isActive: true,
      isVisible: true,
      tabCount: 2,
    }
    const hiddenWindow = {
      ...window,
      windowId: 8,
      isActive: false,
      isVisible: false,
    }
    const session = {
      windows: {
        list: async () => {
          calls.push({ method: 'list' })
          return [window]
        },
        create: async (args?: { hidden?: boolean }) => {
          calls.push({ method: 'create', args })
          return args?.hidden ? hiddenWindow : window
        },
        close: async (windowId: number) => {
          calls.push({ method: 'close', args: windowId })
        },
        activate: async (windowId: number) => {
          calls.push({ method: 'activate', args: windowId })
        },
        setVisibility: async (
          windowId: number,
          args: { visible: boolean; activate?: boolean },
        ) => {
          calls.push({ method: 'setVisibility', args: { windowId, ...args } })
          return {
            previousWindowId: windowId,
            newWindowId: 9,
            replaced: true,
            window: { ...window, windowId: 9, isVisible: args.visible },
          }
        },
      },
      pages: {
        list: async () => [],
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)
    const handler = fake.handlers.get('windows')

    const list = await handler?.({ action: 'list' })
    expect(list?.isError).toBeFalsy()
    expect(list?.structuredContent).toEqual({
      action: 'list',
      windows: [window],
      count: 1,
    })
    expect(list?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('Window 7 (normal, 2 tabs) [ACTIVE]'),
      }),
    ])

    const create = await handler?.({ action: 'create', hidden: true })
    expect(create?.structuredContent).toEqual({
      action: 'create',
      window: hiddenWindow,
    })

    const close = await handler?.({ action: 'close', windowId: 7 })
    expect(close?.structuredContent).toEqual({ action: 'close', windowId: 7 })

    const activate = await handler?.({ action: 'activate', windowId: 8 })
    expect(activate?.structuredContent).toEqual({
      action: 'activate',
      windowId: 8,
    })

    const visibility = await handler?.({
      action: 'set_visibility',
      windowId: 8,
      visible: true,
      activate: false,
    })
    expect(visibility?.structuredContent).toEqual({
      action: 'set_visibility',
      previousWindowId: 8,
      newWindowId: 9,
      replaced: true,
      window: { ...window, windowId: 9, isVisible: true },
    })

    expect(calls).toEqual([
      { method: 'list' },
      { method: 'create', args: { hidden: true } },
      { method: 'close', args: 7 },
      { method: 'activate', args: 8 },
      {
        method: 'setVisibility',
        args: { windowId: 8, visible: true, activate: false },
      },
    ])
  })

  it('returns clear errors for invalid windows actions', async () => {
    const fake = createFakeServer()
    const session = {
      windows: {},
      pages: {
        list: async () => [],
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)
    const handler = fake.handlers.get('windows')

    const close = await handler?.({ action: 'close' })
    expect(close?.isError).toBe(true)
    expect(close?.content).toEqual([
      expect.objectContaining({
        text: 'windows close: windowId is required.',
      }),
    ])

    const visibilityWindow = await handler?.({
      action: 'set_visibility',
      visible: true,
    })
    expect(visibilityWindow?.isError).toBe(true)
    expect(visibilityWindow?.content).toEqual([
      expect.objectContaining({
        text: 'windows set_visibility: windowId is required.',
      }),
    ])

    const visibilityState = await handler?.({
      action: 'set_visibility',
      windowId: 7,
    })
    expect(visibilityState?.isError).toBe(true)
    expect(visibilityState?.content).toEqual([
      expect.objectContaining({
        text: 'windows set_visibility: visible is required.',
      }),
    ])
  })

  it('applies scoped defaults when opening a new tab', async () => {
    const fake = createFakeServer()
    const calls: Array<{
      url: string
      opts?: {
        background?: boolean
        hidden?: boolean
        windowId?: number
        tabGroupId?: string
      }
    }> = []
    const session = {
      pages: {
        newPage: async (
          url: string,
          opts?: {
            background?: boolean
            hidden?: boolean
            windowId?: number
            tabGroupId?: string
          },
        ) => {
          calls.push({ url, opts })
          return 42
        },
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session, {
      defaultWindowId: 7,
      defaultTabGroupId: 'group-a',
    })

    const result = await fake.handlers.get('tabs')?.({
      action: 'new',
      url: 'https://example.com',
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({ page: 42 })
    expect(calls).toEqual([
      {
        url: 'https://example.com',
        opts: {
          background: true,
          hidden: false,
          windowId: 7,
          tabGroupId: 'group-a',
        },
      },
    ])
  })

  it('evaluates page-context JavaScript through the page session', async () => {
    const fake = createFakeServer()
    const evaluateCalls: Array<Record<string, unknown>> = []
    const session = {
      pages: {
        getSession: async () => ({
          session: {
            Runtime: {
              evaluate: async (params: Record<string, unknown>) => {
                evaluateCalls.push(params)
                return { result: { value: 'page-value' } }
              },
            },
          },
        }),
        getInfo: () => ({ url: 'https://example.com/evaluate' }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('evaluate')?.({
      page: 3,
      code: 'return document.title',
      timeout: 1234,
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({ page: 3, value: 'page-value' })
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('[UNTRUSTED_PAGE_CONTENT'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('page-value'),
      }),
    ])
    expect(evaluateCalls).toHaveLength(1)
    expect(evaluateCalls[0]).toMatchObject({
      awaitPromise: true,
      returnByValue: true,
      timeout: 1234,
      userGesture: true,
    })
    expect(String(evaluateCalls[0]?.expression)).toContain(
      'return document.title',
    )
  })

  it('defaults wait timeouts to two seconds', async () => {
    const fake = createFakeServer()
    const session = {
      pages: {
        getSession: async () => ({
          session: {
            Runtime: {
              evaluate: async () => ({ result: { value: false } }),
            },
          },
        }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const originalNow = Date.now
    let nowCalls = 0
    Date.now = () => (nowCalls++ === 0 ? 0 : Number.MAX_SAFE_INTEGER)
    try {
      const result = await fake.handlers.get('wait')?.({
        page: 3,
        for: 'text',
        value: 'ready',
      })

      expect(result?.isError).toBeFalsy()
      expect(result?.structuredContent).toEqual({ matched: false })
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: 'timed out after 2000ms waiting for text',
        }),
      ])
    } finally {
      Date.now = originalNow
    }

    const inputSchema = fake.configs.get('wait')?.inputSchema as
      | { timeout?: { description?: string } }
      | undefined
    expect(inputSchema?.timeout?.description).toContain('default 2000')
  })

  it('runs server-runtime JavaScript against the browser session', async () => {
    const fake = createFakeServer()
    const session = {
      pages: {
        list: async () => [
          { pageId: 7, url: 'https://example.com', title: 'Example' },
        ],
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('run')?.({
      code: `
const pages = await browser.pages.list()
console.log('pages', pages.length)
console.warn({ pageId: pages[0].pageId })
return { title: pages[0].title }
`,
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({
      ok: true,
      value: { title: 'Example' },
      logs: ['pages 1', 'warn: {\n  "pageId": 7\n}'],
    })
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('ok\nreturn:'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('"title": "Example"'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('logs:\npages 1\nwarn:'),
      }),
    ])
  })

  it('keeps run structured values JSON-safe', async () => {
    const fake = createFakeServer()
    const session = { pages: {} } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('run')?.({
      code: `
const value = { id: 1n }
value.self = value
return value
`,
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({
      ok: true,
      value: { id: '1', self: '[Circular]' },
      logs: [],
    })
  })

  it('returns run syntax errors without invoking the browser session', async () => {
    const fake = createFakeServer()
    let listCalls = 0
    const session = {
      pages: {
        list: async () => {
          listCalls += 1
          return []
        },
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('run')?.({
      code: 'const = 1',
    })

    expect(result?.isError).toBe(true)
    expect(result?.structuredContent).toBeUndefined()
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('run: syntax error'),
      }),
    ])
    expect(listCalls).toBe(0)
  })

  it('returns run runtime errors with captured logs', async () => {
    const fake = createFakeServer()
    const session = { pages: {} } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('run')?.({
      code: `
console.log('before boom')
throw new Error('boom')
`,
    })

    expect(result?.isError).toBe(true)
    expect(result?.structuredContent).toEqual({
      ok: false,
      logs: ['before boom'],
      error: 'boom',
    })
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('error: boom'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('logs:\nbefore boom'),
      }),
    ])
  })

  it('returns run timeout errors', async () => {
    const fake = createFakeServer()
    const session = { pages: {} } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('run')?.({
      code: `
await new Promise((resolve) => setTimeout(resolve, 50))
return 'late'
`,
      timeout: 1,
    })

    expect(result?.isError).toBe(true)
    expect(result?.structuredContent).toEqual({
      ok: false,
      logs: [],
      error: 'run exceeded 1ms',
    })
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('run exceeded 1ms'),
      }),
    ])
  })

  it('returns a full snapshot when diff sees a URL change', async () => {
    const fake = createFakeServer()
    const session = {
      observe: () => ({
        diff: async () => ({
          changed: true,
          text: '- main\n  - heading "New page"',
          added: 0,
          removed: 0,
          urlChanged: true,
          beforeUrl: 'https://example.com/old',
          afterUrl: 'https://example.com/new',
        }),
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com/new' }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('diff')?.({ page: 1 })

    expect(result?.isError).toBeFalsy()
    const data = result?.structuredContent as
      | {
          added: number
          removed: number
          urlChanged: boolean
          beforeUrl: string
          afterUrl: string
          snapshot: string
        }
      | undefined
    expect(data).toMatchObject({
      added: 0,
      removed: 0,
      urlChanged: true,
      beforeUrl: 'https://example.com/old',
      afterUrl: 'https://example.com/new',
    })
    expect(data?.snapshot).toContain('[UNTRUSTED_PAGE_CONTENT')
    expect(data?.snapshot).toContain('- heading "New page"')
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('URL changed;'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining(
          'returning full current snapshot instead of a diff',
        ),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('[UNTRUSTED_PAGE_CONTENT'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('- heading "New page"'),
      }),
    ])
  })

  it('wraps same-url diffs with the observed current URL', async () => {
    const fake = createFakeServer()
    const session = {
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
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('diff')?.({ page: 1 })

    expect(result?.isError).toBeFalsy()
    const data = result?.structuredContent as
      | { added: number; removed: number; diff: string }
      | undefined
    expect(data).toMatchObject({ added: 1, removed: 0 })
    expect(data?.diff).toContain('origin=https://example.com/current')
    expect(data?.diff).toContain('+   button "Saved" [ref=e1]')
    expect(data?.diff).not.toContain('origin=https://example.com/stale')
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('origin=https://example.com/current'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.not.stringContaining('origin=https://example.com/stale'),
      }),
    ])
  })

  it('reports an unchanged diff with a changed:false discriminator', async () => {
    const fake = createFakeServer()
    const session = {
      observe: () => ({
        diff: async () => ({ changed: false, text: '', added: 0, removed: 0 }),
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com' }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('diff')?.({ page: 1 })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({ changed: false })
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: 'no change since last snapshot',
      }),
    ])
  })

  it('returns old-threshold-sized direct diffs inline', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const inlineDiff = Array.from(
        { length: 2001 },
        (_, i) => `word-${i}`,
      ).join(' ')
      const session = {
        observe: () => ({
          diff: async () => ({
            changed: true,
            text: inlineDiff,
            added: 2001,
            removed: 0,
            afterUrl: 'https://example.com/large',
          }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large' }),
        },
      } as unknown as BrowserSession

      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('diff')?.({ page: 1 })

      expect(result?.isError).toBeFalsy()
      const data = result?.structuredContent as
        | {
            added: number
            removed: number
            diff: string
          }
        | undefined
      expect(data).toMatchObject({
        added: 2001,
        removed: 0,
      })
      expect(data?.diff).toContain('word-2000')
      expect(data?.diff).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(JSON.stringify(result?.structuredContent)).not.toContain('path')
      expect(JSON.stringify(result?.structuredContent)).not.toContain(
        'writtenToFile',
      )
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('word-2000'),
        }),
      ])
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('[UNTRUSTED_PAGE_CONTENT'),
        }),
      ])
    })
  })

  it('writes large direct diffs to a BrowserOS output markdown file', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const firstMarker = 'first-diff-node'
      const lastMarker = 'last-diff-node'
      const largeDiff = `${firstMarker}\n${'x'.repeat(30_001)}\n${lastMarker}`
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
        },
      } as unknown as BrowserSession

      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('diff')?.({ page: 1 })
      const text = textOf(result)

      expect(result?.isError).toBeFalsy()
      const data = result?.structuredContent as
        | {
            added: number
            removed: number
            truncated: boolean
            tokenEstimate: number
            path: string
            contentLength: number
            writtenToFile: boolean
            diff: string
          }
        | undefined
      expect(data).toMatchObject({
        added: 1,
        removed: 0,
        truncated: true,
        writtenToFile: true,
      })
      expect(data?.tokenEstimate).toBeGreaterThan(10_000)
      const savedPath = data?.path
      await expectBrowserToolOutputPath(savedPath)
      expect(savedPath?.endsWith('.md')).toBe(true)
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('estimated tokens'),
        }),
      ])
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining(savedPath ?? ''),
        }),
      ])
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.not.stringContaining(lastMarker),
        }),
      ])
      expect(text).toContain('Showing the first 5000 estimated tokens inline')
      expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(text).toContain(firstMarker)
      const savedContent = readFileSync(savedPath ?? '', 'utf8')
      expect(savedContent).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(savedContent).toContain(lastMarker)
      expect(data?.diff).toBe(savedContent)
      expect(data?.contentLength).toBe(savedContent.length)
    })
  })

  it('returns a full snapshot when act readback sees a URL change', async () => {
    const fake = createFakeServer()
    const calls: string[] = []
    const session = {
      input: () => ({
        click: async () => calls.push('click'),
      }),
      observe: () => ({
        diff: async () => ({
          changed: true,
          text: '- main\n  - heading "Destination"',
          added: 0,
          removed: 0,
          urlChanged: true,
          beforeUrl: 'https://example.com/start',
          afterUrl: 'https://example.com/destination',
        }),
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com/destination' }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('act')?.({
      page: 1,
      kind: 'click',
      ref: 'e1',
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({
      kind: 'click',
      changed: true,
      urlChanged: true,
      beforeUrl: 'https://example.com/start',
      afterUrl: 'https://example.com/destination',
    })
    expect(result?.content).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('ok (click)'),
        }),
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('[Page 1 diff]'),
        }),
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('URL changed;'),
        }),
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining(
            'returning full current snapshot instead of a diff',
          ),
        }),
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('[UNTRUSTED_PAGE_CONTENT'),
        }),
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('- heading "Destination"'),
        }),
      ]),
    )
    expect(calls).toEqual(['click'])
  })

  it('writes large URL-change act readbacks to a BrowserOS output file', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const firstMarker = 'destination-first-node'
      const lastMarker = 'destination-final-node'
      const largeSnapshot = `${firstMarker}\n${'x'.repeat(30_001)}\n${lastMarker}`
      const session = {
        input: () => ({
          click: async () => undefined,
        }),
        observe: () => ({
          diff: async () => ({
            changed: true,
            text: largeSnapshot,
            added: 0,
            removed: 0,
            urlChanged: true,
            beforeUrl: 'https://example.com/start',
            afterUrl: 'https://example.com/destination',
          }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/destination' }),
        },
      } as unknown as BrowserSession

      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('act')?.({
        page: 1,
        kind: 'click',
        ref: 'e1',
      })

      expect(result?.isError).toBeFalsy()
      const text = textOf(result)
      const savedPath = text.match(/saved to: (.+\.md)/)?.[1]
      expect(result?.structuredContent).toEqual({
        kind: 'click',
        changed: true,
        urlChanged: true,
        beforeUrl: 'https://example.com/start',
        afterUrl: 'https://example.com/destination',
      })
      expect(result?.content).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            type: 'text',
            text: expect.stringContaining('URL changed'),
          }),
          expect.objectContaining({
            type: 'text',
            text: expect.stringContaining('full current snapshot is'),
          }),
          expect.objectContaining({
            type: 'text',
            text: expect.stringContaining('estimated tokens'),
          }),
          expect.objectContaining({
            type: 'text',
            text: expect.stringContaining('saved to:'),
          }),
        ]),
      )
      expect(text).not.toContain(lastMarker)
      expect(text).toContain('Showing the first 5000 estimated tokens inline')
      expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(text).toContain(firstMarker)
      await expectBrowserToolOutputPath(savedPath)
      expect(readFileSync(savedPath ?? '', 'utf8')).toContain(lastMarker)
    })
  })

  it('appends diff output after successful act mutations', async () => {
    const fake = createFakeServer()
    const calls: string[] = []
    const session = {
      input: () => ({
        click: async () => calls.push('click'),
      }),
      observe: () => ({
        diff: async () => {
          calls.push('diff')
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
        getInfo: () => ({ url: 'https://example.com/current' }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('act')?.({
      page: 1,
      kind: 'click',
      ref: 'e1',
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({
      kind: 'click',
      changed: true,
    })
    expect(result?.content).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('ok (click)'),
        }),
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('[Page 1 diff]'),
        }),
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('+   button "Saved" [ref=e1]'),
        }),
      ]),
    )
    expect(calls).toEqual(['click', 'diff'])
  })

  it('caps page-context JavaScript timeouts', async () => {
    const fake = createFakeServer()
    const evaluateCalls: Array<Record<string, unknown>> = []
    const session = {
      pages: {
        getSession: async () => ({
          session: {
            Runtime: {
              evaluate: async (params: Record<string, unknown>) => {
                evaluateCalls.push(params)
                return { result: { value: 'ok' } }
              },
            },
          },
        }),
        getInfo: () => ({ url: 'https://example.com/run' }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('evaluate')?.({
      page: 3,
      code: 'return true',
      timeout: 120_000,
    })

    expect(result?.isError).toBeFalsy()
    expect(evaluateCalls[0]?.timeout).toBe(30_000)
  })

  it('caps large read results and writes the full content to a BrowserOS output file', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const largeText = 'x'.repeat(
        TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS + 1,
      )
      const session = {
        pages: {
          getSession: async () => ({
            session: {
              Runtime: {
                evaluate: async () => ({ result: { value: largeText } }),
              },
            },
          }),
          getInfo: () => ({ url: 'https://example.com' }),
        },
      } as unknown as BrowserSession

      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('read')?.({
        page: 1,
        format: 'text',
      })

      expect(result?.isError).toBeFalsy()
      expect(result?.structuredContent).toMatchObject({
        page: 1,
        format: 'text',
        contentLength: largeText.length,
        writtenToFile: true,
      })
      expect(result?.content).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            type: 'text',
            text: expect.stringContaining('Content truncated'),
          }),
        ]),
      )
      const data = result?.structuredContent as { path?: string } | undefined
      const resultText = (result?.content[0] as { text?: string } | undefined)
        ?.text
      expect(resultText).toBeDefined()
      expect(data?.path).toBeDefined()
      const endMarkerIndex =
        resultText?.indexOf('[END_UNTRUSTED_PAGE_CONTENT') ?? -1
      const pathIndex = resultText?.indexOf(data?.path ?? '') ?? -1
      expect(pathIndex).toBeGreaterThan(endMarkerIndex)
      await expectBrowserToolOutputPath(data?.path)
      const savedContent = readFileSync(data?.path ?? '', 'utf8')
      expect(savedContent).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(savedContent).toContain('[END_UNTRUSTED_PAGE_CONTENT')
      expect(savedContent).toContain(largeText)
    })
  })

  it('prints a page to a BrowserOS output PDF file', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const pdfBytes = Buffer.from('%PDF-1.4 fake pdf', 'utf8')
      const printCalls: Array<Record<string, unknown>> = []
      const session = {
        pages: {
          getSession: async () => ({
            session: {
              Page: {
                printToPDF: async (params: Record<string, unknown>) => {
                  printCalls.push(params)
                  return { data: pdfBytes.toString('base64') }
                },
              },
            },
          }),
        },
      } as unknown as BrowserSession

      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('pdf')?.({
        page: 1,
        landscape: true,
        background: false,
      })
      const upstreamOptions = await fake.handlers.get('pdf')?.({
        page: 1,
        printBackground: false,
        preferCSSPageSize: true,
      })

      expect(result?.isError).toBeFalsy()
      expect(upstreamOptions?.isError).toBeFalsy()
      expect(printCalls).toEqual([
        { landscape: true, printBackground: false, preferCSSPageSize: false },
        { landscape: false, printBackground: false, preferCSSPageSize: true },
      ])
      const data = result?.structuredContent as
        | { page?: number; path?: string; bytes?: number }
        | undefined
      expect(data).toMatchObject({ page: 1, bytes: pdfBytes.length })
      const savedPath = data?.path
      await expectBrowserToolOutputPath(savedPath)
      expect(savedPath?.endsWith('.pdf')).toBe(true)
      expect(readFileSync(savedPath ?? '')).toEqual(pdfBytes)
    })
  })

  it('downloads a file via a ref click into the BrowserOS output dir', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const behaviors: string[] = []
      const clicks: string[] = []
      type DownloadHandler = (params: Record<string, unknown>) => void
      const handlers: Record<string, DownloadHandler> = {}
      const session = {
        input: () => ({
          click: async (ref: string) => {
            clicks.push(ref)
            handlers.downloadWillBegin?.({
              guid: 'g1',
              suggestedFilename: 'report.csv',
            })
            handlers.downloadProgress?.({ guid: 'g1', state: 'completed' })
          },
        }),
        pages: {
          getSession: async () => ({
            session: {
              Page: {
                setDownloadBehavior: async (params: { behavior: string }) => {
                  behaviors.push(params.behavior)
                },
                on: (event: string, handler: DownloadHandler) => {
                  handlers[event] = handler
                  return () => {
                    delete handlers[event]
                  }
                },
              },
            },
          }),
        },
      } as unknown as BrowserSession

      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('download')?.({
        page: 1,
        ref: 'e12',
      })

      expect(result?.isError).toBeFalsy()
      expect(clicks).toEqual(['e12'])
      expect(behaviors).toEqual(['allow', 'default'])
      const data = result?.structuredContent as
        | { page?: number; ref?: string; path?: string; filename?: string }
        | undefined
      expect(data).toMatchObject({
        page: 1,
        ref: 'e12',
        filename: 'report.csv',
      })
      const outputDir = await getToolOutputDir()
      expect(realpathSync(dirname(data?.path ?? ''))).toContain(
        realpathSync(outputDir),
      )
      expect(data?.path?.endsWith('report.csv')).toBe(true)
    })
  })

  it('uploads files into a ref-resolved file input', async () => {
    const fake = createFakeServer()
    const uploads: Array<{ ref: string; files: string[] }> = []
    const session = {
      input: () => ({
        uploadFile: async (ref: string, files: string[]) => {
          uploads.push({ ref, files })
        },
      }),
      pages: {},
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)
    expect(fake.handlers.has('upload')).toBe(true)

    const multiple = await fake.handlers.get('upload')?.({
      page: 1,
      ref: 'e12',
      files: ['/tmp/a.txt', '/tmp/b.txt'],
    })
    const single = await fake.handlers.get('upload')?.({
      page: 1,
      ref: 'e13',
      file: '/tmp/c.txt',
    })

    expect(multiple?.isError).toBeFalsy()
    expect(single?.isError).toBeFalsy()
    expect(uploads).toEqual([
      { ref: 'e12', files: ['/tmp/a.txt', '/tmp/b.txt'] },
      { ref: 'e13', files: ['/tmp/c.txt'] },
    ])
    expect(single?.structuredContent).toEqual({
      page: 1,
      ref: 'e13',
      files: ['/tmp/c.txt'],
      uploaded: 1,
    })
  })

  it('keeps small snapshots inline with the existing structured content', async () => {
    const fake = createFakeServer()
    const session = {
      observe: () => ({
        snapshot: async () => ({ text: '- button "Save" [ref=e1]' }),
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com/small' }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('snapshot')?.({ page: 2 })

    expect(result?.isError).toBeFalsy()
    const data = result?.structuredContent as
      | { page: number; snapshot: string }
      | undefined
    expect(data).toMatchObject({ page: 2 })
    expect(data?.snapshot).toContain('[UNTRUSTED_PAGE_CONTENT')
    expect(data?.snapshot).toContain('- button "Save" [ref=e1]')
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('[UNTRUSTED_PAGE_CONTENT'),
      }),
    ])
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('- button "Save" [ref=e1]'),
      }),
    ])
    expect(JSON.stringify(result?.structuredContent)).not.toContain('path')
  })

  it('returns word-threshold-only snapshots inline', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const inlineSnapshot = `${'x '.repeat(15_001)}last-node`
      const session = {
        observe: () => ({
          snapshot: async () => ({ text: inlineSnapshot }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large' }),
        },
      } as unknown as BrowserSession
      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('snapshot')?.({ page: 4 })

      expect(result?.isError).toBeFalsy()
      const data = result?.structuredContent as
        | {
            page: number
            snapshot: string
          }
        | undefined
      expect(data).toMatchObject({
        page: 4,
      })
      expect(data?.snapshot).toContain('last-node')
      expect(data?.snapshot).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(JSON.stringify(result?.structuredContent)).not.toContain('path')
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('last-node'),
        }),
      ])
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('[UNTRUSTED_PAGE_CONTENT'),
        }),
      ])
    })
  })

  it('writes very large snapshots to a BrowserOS output markdown file', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const firstMarker = 'first-node'
      const lastMarker = 'last-node'
      const largeSnapshot = `${firstMarker}\n${'x '.repeat(23_000)}${lastMarker}`
      expect(largeSnapshot.length).toBeLessThan(50_000)
      const session = {
        observe: () => ({
          snapshot: async () => ({ text: largeSnapshot }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/large' }),
        },
      } as unknown as BrowserSession
      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('snapshot')?.({ page: 4 })
      const text = textOf(result)

      expect(result?.isError).toBeFalsy()
      const data = result?.structuredContent as
        | {
            page: number
            path: string
            contentLength: number
            tokenEstimate: number
            writtenToFile: boolean
            snapshot: string
          }
        | undefined
      expect(data).toMatchObject({
        page: 4,
        writtenToFile: true,
      })
      expect(data?.tokenEstimate).toBeGreaterThan(15_000)
      const savedPath = data?.path
      await expectBrowserToolOutputPath(savedPath)
      expect(savedPath?.endsWith('.md')).toBe(true)
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining('Large snapshot'),
        }),
      ])
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining(savedPath ?? ''),
        }),
      ])
      expect(text).toContain('Showing the first 5000 estimated tokens inline')
      expect(text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(text).toContain(firstMarker)
      expect(text).not.toContain(lastMarker)
      expect(existsSync(savedPath ?? '')).toBe(true)

      const savedContent = readFileSync(savedPath ?? '', 'utf8')
      expect(savedContent).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(savedContent).toContain('[END_UNTRUSTED_PAGE_CONTENT')
      expect(savedContent).toContain(lastMarker)
      expect(data?.snapshot).toBe(savedContent)
      expect(data?.contentLength).toBe(savedContent.length)
    })
  })

  it('writes very long unbroken snapshots to a BrowserOS output markdown file', async () => {
    await withBrowserosDir(async () => {
      const fake = createFakeServer()
      const largeSnapshot = 'x'.repeat(45_001)
      const session = {
        observe: () => ({
          snapshot: async () => ({ text: largeSnapshot }),
        }),
        pages: {
          getInfo: () => ({ url: 'https://example.com/long-token' }),
        },
      } as unknown as BrowserSession
      registerBrowserTools(fake.server as never, session)

      const result = await fake.handlers.get('snapshot')?.({ page: 5 })
      const data = result?.structuredContent as
        | {
            path: string
            tokenEstimate: number
            writtenToFile: boolean
          }
        | undefined

      expect(result?.isError).toBeFalsy()
      expect(data).toMatchObject({
        writtenToFile: true,
      })
      expect(data?.tokenEstimate).toBeGreaterThan(15_000)
      const savedPath = data?.path
      expect(result?.content).toEqual([
        expect.objectContaining({
          type: 'text',
          text: expect.stringContaining(savedPath ?? ''),
        }),
      ])
      await expectBrowserToolOutputPath(savedPath)
      expect(readFileSync(savedPath ?? '', 'utf8')).toContain(largeSnapshot)
    })
  })

  it('returns read errors for page-side exceptions', async () => {
    const fake = createFakeServer()
    const session = {
      pages: {
        getSession: async () => ({
          session: {
            Runtime: {
              evaluate: async () => ({
                exceptionDetails: { text: 'SyntaxError: invalid selector' },
              }),
            },
          },
        }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    const result = await fake.handlers.get('read')?.({
      page: 1,
      format: 'text',
      selector: '[',
    })

    expect(result?.isError).toBe(true)
    expect(result?.content).toEqual([
      { type: 'text', text: 'read: SyntaxError: invalid selector' },
    ])
  })

  it('routes compact tools through browser session APIs', async () => {
    const fake = createFakeServer()
    const calls: string[] = []
    const input = {
      click: async () => calls.push('click'),
      clickAt: async () => calls.push('clickAt'),
      type: async () => calls.push('type'),
      typeAt: async () => calls.push('typeAt'),
      fill: async () => calls.push('fill'),
      press: async () => calls.push('press'),
      hover: async () => calls.push('hover'),
      hoverAt: async () => calls.push('hoverAt'),
      focus: async (ref: string) => calls.push(`focus:${ref}`),
      check: async (ref: string) => calls.push(`check:${ref}`),
      uncheck: async (ref: string) => calls.push(`uncheck:${ref}`),
      selectOption: async () => calls.push('selectOption'),
      scroll: async () => calls.push('scroll'),
      drag: async (source: string, target: string) =>
        calls.push(`drag:${source}->${target}`),
      dragAt: async () => calls.push('dragAt'),
    }
    const session = {
      nav: () => ({
        goto: async () => calls.push('goto'),
        back: async () => calls.push('back'),
        forward: async () => calls.push('forward'),
        reload: async () => calls.push('reload'),
      }),
      observe: () => ({
        snapshot: async () => {
          calls.push('snapshot')
          return { text: '- button "Save" [ref=e1]' }
        },
        diff: async () => {
          calls.push('diff')
          return { changed: true, text: '+ saved', added: 1, removed: 0 }
        },
      }),
      input: () => input,
      screenshot: async () => ({
        data: 'image-data',
        mimeType: 'image/jpeg',
        annotations: [],
      }),
      pages: {
        getInfo: () => ({ url: 'https://example.com' }),
        refresh: async () => ({ url: 'https://example.com' }),
        getSession: async () => ({
          session: {
            Runtime: {
              evaluate: async () => ({ result: { value: true } }),
            },
            Page: {
              getLayoutMetrics: async () => ({
                layoutViewport: {
                  pageX: 0,
                  pageY: 0,
                  clientWidth: 1024,
                  clientHeight: 768,
                },
                cssLayoutViewport: {
                  pageX: 0,
                  pageY: 0,
                  clientWidth: 1024,
                  clientHeight: 768,
                },
              }),
              captureScreenshot: async () => ({ data: 'image-data' }),
            },
          },
        }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(fake.server as never, session)

    expect(
      await fake.handlers.get('navigate')?.({
        page: 1,
        action: 'url',
        url: 'https://example.com',
      }),
    ).toMatchObject({ isError: undefined })
    expect(await fake.handlers.get('snapshot')?.({ page: 1 })).toMatchObject({
      structuredContent: { page: 1 },
    })
    expect(await fake.handlers.get('diff')?.({ page: 1 })).toMatchObject({
      structuredContent: { added: 1, removed: 0 },
    })
    expect(
      await fake.handlers.get('grep')?.({
        page: 1,
        pattern: 'save',
        over: 'ax',
      }),
    ).toMatchObject({ structuredContent: { count: 1 } })
    expect(await fake.handlers.get('screenshot')?.({ page: 1 })).toMatchObject({
      content: [{ type: 'image', data: 'image-data', mimeType: 'image/jpeg' }],
    })
    expect(
      await fake.handlers.get('wait')?.({
        page: 1,
        for: 'selector',
        value: '#ready',
      }),
    ).toMatchObject({ structuredContent: { matched: true } })
    expect(
      await fake.handlers.get('act')?.({
        page: 1,
        kind: 'click',
        ref: 'e1',
      }),
    ).toMatchObject({ structuredContent: { kind: 'click', changed: true } })
    await fake.handlers.get('act')?.({
      page: 1,
      kind: 'focus',
      ref: 'e1',
    })
    await fake.handlers.get('act')?.({
      page: 1,
      kind: 'check',
      ref: 'e2',
    })
    await fake.handlers.get('act')?.({
      page: 1,
      kind: 'uncheck',
      ref: 'e3',
    })
    await fake.handlers.get('act')?.({
      page: 1,
      kind: 'drag',
      ref: 'e4',
      targetRef: 'e5',
    })
    expect(calls).toEqual([
      'goto',
      'snapshot',
      'snapshot',
      'diff',
      'snapshot',
      'click',
      'diff',
      'focus:e1',
      'diff',
      'check:e2',
      'diff',
      'uncheck:e3',
      'diff',
      'drag:e4->e5',
      'diff',
    ])
  })
})

describe('buildBrowserToolSet', () => {
  it('builds the compact browser tool surface', () => {
    const session = { pages: {} } as unknown as BrowserSession
    const tools = buildBrowserToolSet(session)

    expect(tools.tabs).toBeDefined()
    expect(tools.new_page).toBeUndefined()
    expect(Object.keys(tools)).toEqual(BROWSER_TOOLS.map((t) => t.name))
  })

  it('records generated output paths from AI SDK browser tools', async () => {
    await withBrowserosDir(async () => {
      const outputFileAccess = createBrowserOutputFileAccess()
      const largeText = 'x'.repeat(
        TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS + 1,
      )
      const session = {
        pages: {
          getSession: async () => ({
            session: {
              Runtime: {
                evaluate: async () => ({ result: { value: largeText } }),
              },
            },
          }),
          getInfo: () => ({ url: 'https://example.com' }),
        },
      } as unknown as BrowserSession
      const tools = buildBrowserToolSet(session, { outputFileAccess })

      const result = await tools.read.execute?.({ page: 1, format: 'text' }, {
        abortSignal: new AbortController().signal,
      } as never)
      const text =
        (result as { content?: Array<{ type: string; text: string }> }).content
          ?.filter((item) => item.type === 'text')
          .map((item) => item.text)
          .join('\n') ?? ''
      const savedPath = text.match(/saved to: (.+\.txt)/)?.[1]

      expect(savedPath).toBeTruthy()
      await expectBrowserToolOutputPath(savedPath)
      expect(outputFileAccess.paths.has(savedPath ?? '')).toBe(true)
    })
  })

  it('allows no-workspace filesystem readback for AI SDK downloads', async () => {
    await withBrowserosDir(async () => {
      const outputFileAccess = createBrowserOutputFileAccess()
      const behaviors: string[] = []
      const clicks: string[] = []
      let downloadDir = ''
      type DownloadHandler = (params: Record<string, unknown>) => void
      const handlers: Record<string, DownloadHandler> = {}
      const session = {
        input: () => ({
          click: async (ref: string) => {
            clicks.push(ref)
            writeFileSync(
              join(downloadDir, 'report.csv'),
              'name,value\nbrowseros,1\n',
            )
            handlers.downloadWillBegin?.({
              guid: 'g1',
              suggestedFilename: 'report.csv',
            })
            handlers.downloadProgress?.({ guid: 'g1', state: 'completed' })
          },
        }),
        pages: {
          getSession: async () => ({
            session: {
              Page: {
                setDownloadBehavior: async (params: {
                  behavior: string
                  downloadPath?: string
                }) => {
                  behaviors.push(params.behavior)
                  if (params.downloadPath) downloadDir = params.downloadPath
                },
                on: (event: string, handler: DownloadHandler) => {
                  handlers[event] = handler
                  return () => {
                    delete handlers[event]
                  }
                },
              },
            },
          }),
        },
      } as unknown as BrowserSession
      const tools = buildBrowserToolSet(session, { outputFileAccess })
      const readTool = createReadTool(undefined, {
        allowedOutputPaths: outputFileAccess.paths,
      }) as unknown as {
        execute(params: Record<string, unknown>): Promise<{ text: string }>
      }

      const downloadResult = await tools.download.execute?.(
        { page: 1, ref: 'e12' },
        { abortSignal: new AbortController().signal } as never,
      )
      const text =
        (
          downloadResult as { content?: Array<{ type: string; text: string }> }
        ).content
          ?.filter((item) => item.type === 'text')
          .map((item) => item.text)
          .join('\n') ?? ''
      const savedPath = text.match(/to: (.+report\.csv)/)?.[1]
      const readResult = await readTool.execute({ path: savedPath })

      expect(clicks).toEqual(['e12'])
      expect(behaviors).toEqual(['allow', 'default'])
      expect(savedPath).toBeTruthy()
      expect(outputFileAccess.paths.has(savedPath ?? '')).toBe(true)
      expect(readResult.text).toContain('[UNTRUSTED_PAGE_CONTENT')
      expect(readResult.text).toContain('[END_UNTRUSTED_PAGE_CONTENT')
      expect(readResult.text).toContain('browseros,1')
    })
  })

  it('allows chat mode to read tabs without allowing tab mutation', async () => {
    expect(CHAT_MODE_ALLOWED_TOOLS.has('tabs')).toBe(true)
    const calls: string[] = []
    const activePage = {
      pageId: 1,
      targetId: 'target-1',
      tabId: 11,
      url: 'https://example.com',
      title: 'Example',
      isActive: true,
      isLoading: false,
      loadProgress: 1,
      isPinned: false,
      isHidden: false,
    }
    const session = {
      pages: {
        list: async () => [activePage],
        getActive: async () => activePage,
        newPage: async () => {
          calls.push('newPage')
          return 2
        },
      },
    } as unknown as BrowserSession
    const tools = buildBrowserToolSet(session, { readOnly: true })

    const listResult = await tools.tabs.execute?.({ action: 'list' }, {
      abortSignal: new AbortController().signal,
    } as never)
    const activeResult = await tools.tabs.execute?.({ action: 'active' }, {
      abortSignal: new AbortController().signal,
    } as never)
    const newResult = await tools.tabs.execute?.(
      { action: 'new', url: 'https://example.com' },
      { abortSignal: new AbortController().signal } as never,
    )

    expect(listResult).toMatchObject({ isError: false })
    expect(activeResult).toMatchObject({ isError: false })
    expect(newResult).toMatchObject({ isError: true })
    expect(calls).toEqual([])
  })

  it('propagates AI SDK abort signals into browser tools', async () => {
    const session = { pages: {} } as unknown as BrowserSession
    const tools = buildBrowserToolSet(session)
    const controller = new AbortController()
    controller.abort(new Error('cancelled'))

    let caught: unknown
    try {
      await tools.wait.execute?.({ page: 1, for: 'time', value: '1000' }, {
        abortSignal: controller.signal,
      } as never)
    } catch (error) {
      caught = error
    }

    expect(caught).toBeInstanceOf(Error)
    expect((caught as Error).message).toBe('cancelled')
  })

  it('stops awaiting in-flight browser tools when aborted', async () => {
    const slowTool = defineTool({
      name: 'slow',
      description: 'Slow test tool.',
      input: z.object({}),
      handler: async () => {
        await new Promise(() => {})
        return textResult('done')
      },
    })
    const controller = new AbortController()

    const pending = executeTool(
      slowTool,
      {},
      {
        session: {} as BrowserSession,
        signal: controller.signal,
      },
    )
    controller.abort(new Error('cancelled'))

    await expect(pending).rejects.toThrow('cancelled')
  })
})
