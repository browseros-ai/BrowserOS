import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createBrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import { registerTools } from '../../../../src/api/services/mcp/register-mcp'
import { logger } from '../../../../src/lib/logger'
import { resetToolRegistrationLogSamplingForTests } from '../../../../src/tools/registration-log-sampling'

type RegisteredHandler = (
  args: Record<string, unknown>,
  extra?: { signal?: AbortSignal },
) => Promise<{
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

function textOf(result: Awaited<ReturnType<RegisteredHandler>> | undefined) {
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
  const browserosDir = await mkdtemp(join(tmpdir(), 'mcp-output-test-'))
  process.env.BROWSEROS_DIR = browserosDir
  try {
    return await run()
  } finally {
    if (previous === undefined) {
      delete process.env.BROWSEROS_DIR
    } else {
      process.env.BROWSEROS_DIR = previous
    }
    await rm(browserosDir, { recursive: true, force: true })
  }
}

describe('registerTools', () => {
  const originalInfo = logger.info
  const originalError = logger.error
  const filesystemToolNames = [
    'filesystem_read',
    'filesystem_write',
    'filesystem_edit',
    'filesystem_bash',
    'filesystem_grep',
    'filesystem_find',
    'filesystem_ls',
  ]
  let infoMessages: unknown[] = []

  beforeEach(() => {
    resetToolRegistrationLogSamplingForTests()
    infoMessages = []
    logger.info = ((message: string) => {
      infoMessages.push(message)
    }) as typeof logger.info
    logger.error = (() => {}) as typeof logger.error
  })

  afterEach(() => {
    logger.info = originalInfo
    logger.error = originalError
    resetToolRegistrationLogSamplingForTests()
  })

  it('registers the browser tools', () => {
    const fake = createFakeServer()

    registerTools(fake.server as never, {
      browserSession: { pages: {} } as unknown as BrowserSession,
      executionDir: '/tmp/browseros-execution',
    })

    expect([...fake.handlers.keys()]).toEqual(BROWSER_TOOLS.map((t) => t.name))
    expect(fake.handlers.size).toBe(BROWSER_TOOLS.length)
  })

  it('registers filesystem tools for remote agent harness requests', () => {
    const fake = createFakeServer()

    registerTools(fake.server as never, {
      browserSession: { pages: {} } as unknown as BrowserSession,
      executionDir: '/tmp/browseros-execution',
      remoteAgentHarness: {
        outputFileAccess: createBrowserOutputFileAccess(),
      },
    })

    expect([...fake.handlers.keys()]).toEqual([
      ...BROWSER_TOOLS.map((t) => t.name),
      ...filesystemToolNames,
    ])
  })

  it('lets remote agent harness read browser-generated output files across MCP registrations', async () => {
    await withBrowserosDir(async () => {
      const outputFileAccess = createBrowserOutputFileAccess()
      const largeText = 'remote harness generated output\n'.repeat(
        Math.ceil(TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS / 32) + 1,
      )
      const browserSession = {
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

      const browserRequest = createFakeServer()
      registerTools(browserRequest.server as never, {
        browserSession,
        executionDir: '/tmp/browseros-execution',
        remoteAgentHarness: { outputFileAccess },
      })

      const browserResult = await browserRequest.handlers.get('read')?.({
        page: 1,
        format: 'text',
      })
      const savedPath = textOf(browserResult).match(/saved to: (.+\.txt)/)?.[1]

      expect(savedPath).toBeTruthy()
      expect(outputFileAccess.paths.has(savedPath ?? '')).toBe(true)

      const filesystemRequest = createFakeServer()
      registerTools(filesystemRequest.server as never, {
        browserSession: { pages: {} } as unknown as BrowserSession,
        executionDir: '/tmp/browseros-execution',
        remoteAgentHarness: { outputFileAccess },
      })

      const readResult = await filesystemRequest.handlers.get(
        'filesystem_read',
      )?.({ path: savedPath })

      expect(readResult?.isError).toBeFalsy()
      expect(textOf(readResult)).toContain('remote harness generated output')

      const otherHarness = createFakeServer()
      registerTools(otherHarness.server as never, {
        browserSession: { pages: {} } as unknown as BrowserSession,
        executionDir: '/tmp/browseros-execution',
        remoteAgentHarness: {
          outputFileAccess: createBrowserOutputFileAccess(),
        },
      })

      const denied = await otherHarness.handlers.get('filesystem_read')?.({
        path: savedPath,
      })

      expect(denied?.isError).toBe(true)
      expect(textOf(denied)).toContain('returned in this session')
    })
  })

  it('samples repeated registration info logs without skipping tool registration', () => {
    for (let i = 0; i < 20; i++) {
      const fake = createFakeServer()
      registerTools(fake.server as never, {
        browserSession: { pages: {} } as unknown as BrowserSession,
        executionDir: '/tmp/browseros-execution',
      })

      if (i === 1) {
        expect([...fake.handlers.keys()]).toEqual(
          BROWSER_TOOLS.map((t) => t.name),
        )
      }
      if (i === 2) {
        expect(fake.handlers.has('tabs')).toBe(true)
        expect(fake.handlers.has('new_page')).toBe(false)
      }
    }

    expect(infoMessages).toHaveLength(2)
    expect(infoMessages).toEqual([
      expect.stringContaining(
        `Registered ${BROWSER_TOOLS.length} browser tools`,
      ),
      expect.stringContaining(
        `Registered ${BROWSER_TOOLS.length} browser tools`,
      ),
    ])
  })

  it('applies scoped defaults to tab creation', async () => {
    const fake = createFakeServer()
    const newPageCalls: Array<{
      url: string
      opts?: {
        background?: boolean
        hidden?: boolean
        tabGroupId?: string
        windowId?: number
      }
    }> = []

    registerTools(fake.server as never, {
      browserSession: {
        pages: {
          newPage: async (
            url: string,
            opts?: {
              background?: boolean
              hidden?: boolean
              tabGroupId?: string
              windowId?: number
            },
          ) => {
            newPageCalls.push({ url, opts })
            return 42
          },
        },
      } as unknown as BrowserSession,
      defaultWindowId: 7,
      defaultTabGroupId: 'group-a',
      executionDir: '/tmp/browseros-execution',
    })

    const result = await fake.handlers.get('tabs')?.({
      action: 'new',
      url: 'https://example.com',
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({ page: 42 })
    expect(newPageCalls).toEqual([
      {
        url: 'https://example.com',
        opts: {
          background: true,
          hidden: false,
          tabGroupId: 'group-a',
          windowId: 7,
        },
      },
    ])
  })
})
