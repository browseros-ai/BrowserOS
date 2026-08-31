import { describe, expect, it, mock } from 'bun:test'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createBrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import type { ActiveConversationRun } from '../../../../src/api/services/conversation-runs'
import { KlavisService } from '../../../../src/api/services/klavis/service'
import { BrowserMcpModule } from '../../../../src/api/services/mcp/browser-mcp-module'
import { createReadTool } from '../../../../src/tools/filesystem/read'

type RegisteredTool = {
  handler: (args: Record<string, unknown>) => Promise<{
    content: Array<{ type: string; text?: string }>
    isError?: boolean
    structuredContent?: unknown
  }>
}

type InspectableMcpServer = {
  _registeredTools: Record<string, RegisteredTool>
}

function inspect(server: unknown): InspectableMcpServer {
  return server as InspectableMcpServer
}

function browserSession(): BrowserSession {
  const pages = [
    {
      pageId: 1,
      targetId: 'target-1',
      tabId: 101,
      url: 'https://example.com',
      title: 'Example',
      isActive: true,
      isLoading: false,
      loadProgress: 1,
      isPinned: false,
      windowId: 7,
    },
  ]

  return {
    pages: {
      list: async () => pages,
      getActive: async () => pages[0],
      getInfo: (pageId: number) => pages.find((page) => page.pageId === pageId),
      getTabId: (pageId: number) =>
        pages.find((page) => page.pageId === pageId)?.tabId,
      newPage: async (url: string) => {
        pages.push({
          ...pages[0],
          pageId: 2,
          targetId: 'target-2',
          tabId: 102,
          url,
          title: 'New',
          isActive: false,
        })
        return 2
      },
    },
    nav: (_pageId: number) => ({
      reload: async () => undefined,
    }),
  } as unknown as BrowserSession
}

describe('BrowserMcpModule', () => {
  it('treats a read-only lease as an authoritative permission ceiling', async () => {
    const { browserMcp: runtime, runs } = moduleFixture()
    const conversationId = crypto.randomUUID()
    const lease = runtime.createLease({
      conversationId,
      readOnly: true,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    runs.start(conversationId, 'read-only-run')

    const server = inspect(
      runtime.createMcpServer({
        leaseToken: lease.token,
        requestedReadOnly: false,
      }),
    )

    expect(Object.keys(server._registeredTools).sort()).toEqual([
      'diff',
      'grep',
      'history',
      'pdf',
      'read',
      'screenshot',
      'snapshot',
      'tabs',
      'wait',
    ])

    const result = await server._registeredTools.tabs.handler({
      action: 'new',
      url: 'https://browseros.com',
    })

    expect(result.isError).toBe(true)
    expect(result.content[0]?.text).toContain('read-only')
  })

  it('lets an external MCP client request the same read-only surface', () => {
    const { browserMcp: runtime } = moduleFixture()

    const server = inspect(runtime.createMcpServer({ requestedReadOnly: true }))

    expect(Object.keys(server._registeredTools)).not.toContain('navigate')
    expect(Object.keys(server._registeredTools)).toContain('tabs')
  })

  it('runs conversation-tab effects for the active run but not for tabs list', async () => {
    const conversationId = crypto.randomUUID()
    const { browserMcp: runtime, runs } = moduleFixture()
    const lease = runtime.createLease({
      conversationId,
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    runs.start(conversationId, 'run-1')
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))

    await server._registeredTools.tabs.handler({ action: 'list' })
    expect(runs.associated).toEqual([])

    await server._registeredTools.tabs.handler({ action: 'active' })
    expect(runs.associated).toEqual([
      { conversationId, runId: 'run-1', tabIds: [101] },
    ])
  })

  it('observes pages touched indirectly through the run browser SDK', async () => {
    const conversationId = crypto.randomUUID()
    const { browserMcp: runtime, runs } = moduleFixture()
    const lease = runtime.createLease({
      conversationId,
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    runs.start(conversationId, 'run-2')
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))

    await server._registeredTools.run.handler({
      code: 'await browser.nav(1).reload(); return "done"',
    })

    expect(runs.associated).toEqual([
      { conversationId, runId: 'run-2', tabIds: [101] },
    ])
  })

  it('groups the first tab created by an agent call', async () => {
    const conversationId = crypto.randomUUID()
    const { addCreatedPages, browserMcp: runtime, runs } = moduleFixture()
    const run = runs.start(conversationId, 'group-run')
    const lease = runtime.createLease({
      conversationId,
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))

    await server._registeredTools.tabs.handler({
      action: 'new',
      url: 'https://browseros.com',
    })

    expect(addCreatedPages).toHaveBeenCalledTimes(1)
    expect(addCreatedPages).toHaveBeenCalledWith(run, [2])
  })

  it('attributes a late tool effect to the run that authorized the call', async () => {
    let finishReload!: () => void
    const reloadBlocked = new Promise<void>((resolve) => {
      finishReload = resolve
    })
    const session = browserSession()
    session.nav = () =>
      ({
        reload: async () => await reloadBlocked,
      }) as ReturnType<BrowserSession['nav']>
    const conversationId = crypto.randomUUID()
    const { browserMcp: runtime, runs } = moduleFixture({ session })
    const lease = runtime.createLease({
      conversationId,
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    runs.start(conversationId, 'run-a')
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))

    const execution = server._registeredTools.run.handler({
      code: 'await browser.nav(1).reload(); return "done"',
    })
    await Promise.resolve()
    runs.start(conversationId, 'run-b')
    finishReload()
    await execution

    expect(runs.associated).toEqual([])
  })

  it('rejects stale internal lease tokens instead of falling back to full access', () => {
    const { browserMcp: runtime } = moduleFixture()

    expect(() =>
      runtime.createMcpServer({ leaseToken: 'stale-token' }),
    ).toThrow('Invalid or expired BrowserOS tool lease')
  })

  it('does not add a per-call lease-liveness guard', async () => {
    const { browserMcp: runtime, runs } = moduleFixture()
    const conversationId = crypto.randomUUID()
    const lease = runtime.createLease({
      conversationId,
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    runs.start(conversationId, 'revoked-run')
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))

    lease.revoke()
    const result = await server._registeredTools.tabs.handler({
      action: 'active',
    })

    expect(result.isError).toBeUndefined()
    expect(() => runtime.createMcpServer({ leaseToken: lease.token })).toThrow(
      'Invalid or expired BrowserOS tool lease',
    )
  })

  it('does not execute through a valid lease outside its active run', async () => {
    const { browserMcp: runtime } = moduleFixture()
    const lease = runtime.createLease({
      conversationId: crypto.randomUUID(),
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))

    const result = await server._registeredTools.tabs.handler({
      action: 'new',
      url: 'https://browseros.com',
    })

    expect(result.isError).toBe(true)
    expect(result.content[0]?.text).toContain('active conversation run')
  })

  it('applies conversation-running and read-only guards to managed connectors', async () => {
    const getUserIntegrations = mock(async () => [])
    const { browserMcp: runtime, runs } = moduleFixture({
      klavis: new KlavisService({
        browserosId: 'browseros-test',
        client: { getUserIntegrations } as never,
      }),
    })
    const conversationId = crypto.randomUUID()
    const lease = runtime.createLease({
      conversationId,
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))

    const inactive =
      await server._registeredTools.connector_mcp_servers.handler({})
    expect(inactive.isError).toBe(true)
    expect(getUserIntegrations).not.toHaveBeenCalled()

    runs.start(conversationId, 'connector-run')
    const active = await server._registeredTools.connector_mcp_servers.handler(
      {},
    )
    expect(active.isError).toBeUndefined()
    expect(getUserIntegrations).toHaveBeenCalledTimes(1)

    runs.finish(conversationId)
    const finished =
      await server._registeredTools.connector_mcp_servers.handler({})
    expect(finished.isError).toBe(true)
    expect(getUserIntegrations).toHaveBeenCalledTimes(1)

    const readOnlyConversationId = crypto.randomUUID()
    const readOnlyLease = runtime.createLease({
      conversationId: readOnlyConversationId,
      readOnly: true,
      outputFileAccess: createBrowserOutputFileAccess(),
    })
    runs.start(readOnlyConversationId, 'read-only-connector-run')
    const readOnlyServer = inspect(
      runtime.createMcpServer({ leaseToken: readOnlyLease.token }),
    )
    const readOnly =
      await readOnlyServer._registeredTools.connector_mcp_servers.handler({})
    expect(readOnly.isError).toBe(true)
    expect(readOnly.content[0]?.text).toContain('read-only')
    expect(getUserIntegrations).toHaveBeenCalledTimes(1)
  })

  it('refreshes mutable browser context without replacing the agent lease', async () => {
    let openedInWindow: number | undefined
    const session = browserSession()
    session.pages.newPage = async (_url, options) => {
      openedInWindow = options?.windowId
      return 2
    }
    const { browserMcp: runtime, runs } = moduleFixture({ session })
    const conversationId = crypto.randomUUID()
    const lease = runtime.createLease({
      conversationId,
      readOnly: false,
      outputFileAccess: createBrowserOutputFileAccess(),
      browserContext: { windowId: 1 },
    })

    lease.updateBrowserContext({ windowId: 22 })
    runs.start(conversationId, 'context-run')
    const server = inspect(runtime.createMcpServer({ leaseToken: lease.token }))
    await server._registeredTools.tabs.handler({
      action: 'new',
      url: 'https://browseros.com',
    })

    expect(openedInWindow).toBe(22)
  })

  it('grants filesystem readback for output created through loopback MCP', async () => {
    const browserosDir = mkdtempSync(join(tmpdir(), 'browser-mcp-module-'))
    const previousBrowserosDir = process.env.BROWSEROS_DIR
    process.env.BROWSEROS_DIR = browserosDir
    try {
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
          getTabId: () => 101,
        },
      } as unknown as BrowserSession
      const outputFileAccess = createBrowserOutputFileAccess()
      const { browserMcp: runtime, runs } = moduleFixture({ session })
      const conversationId = crypto.randomUUID()
      const lease = runtime.createLease({
        conversationId,
        readOnly: false,
        outputFileAccess,
      })
      runs.start(conversationId, 'output-run')
      const server = inspect(
        runtime.createMcpServer({ leaseToken: lease.token }),
      )

      const result = await server._registeredTools.read.handler({
        page: 1,
        format: 'text',
      })
      const text = result.content.map((item) => item.text ?? '').join('\n')
      const savedPath = text.match(/saved to: (.+\.txt)/)?.[1]
      expect(savedPath).toBeTruthy()
      expect(outputFileAccess.paths.has(savedPath ?? '')).toBe(true)

      const filesystemRead = createReadTool(undefined, {
        allowedOutputPaths: outputFileAccess.paths,
      }) as unknown as {
        execute(input: { path: string }): Promise<{ text: string }>
      }
      const readback = await filesystemRead.execute({ path: savedPath ?? '' })
      expect(readback.text).toContain('x'.repeat(100))
    } finally {
      if (previousBrowserosDir === undefined) {
        delete process.env.BROWSEROS_DIR
      } else {
        process.env.BROWSEROS_DIR = previousBrowserosDir
      }
      rmSync(browserosDir, { recursive: true, force: true })
    }
  })

  it('grants filesystem readback for downloads created through loopback MCP', async () => {
    const browserosDir = mkdtempSync(join(tmpdir(), 'browser-tool-download-'))
    const previousBrowserosDir = process.env.BROWSEROS_DIR
    process.env.BROWSEROS_DIR = browserosDir
    try {
      let downloadDir = ''
      type DownloadHandler = (params: Record<string, unknown>) => void
      const handlers: Record<string, DownloadHandler> = {}
      const session = {
        input: () => ({
          click: async () => {
            writeFileSync(
              join(downloadDir, 'report.csv'),
              'name,value\nneo,1\n',
            )
            handlers.downloadWillBegin?.({
              guid: 'download-1',
              suggestedFilename: 'report.csv',
            })
            handlers.downloadProgress?.({
              guid: 'download-1',
              state: 'completed',
            })
          },
        }),
        pages: {
          getSession: async () => ({
            session: {
              Page: {
                setDownloadBehavior: async (params: {
                  downloadPath?: string
                }) => {
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
          getInfo: () => ({ url: 'https://example.com' }),
          getTabId: () => 101,
        },
      } as unknown as BrowserSession
      const outputFileAccess = createBrowserOutputFileAccess()
      const { browserMcp: runtime, runs } = moduleFixture({ session })
      const conversationId = crypto.randomUUID()
      const lease = runtime.createLease({
        conversationId,
        readOnly: false,
        outputFileAccess,
      })
      runs.start(conversationId, 'download-run')
      const server = inspect(
        runtime.createMcpServer({ leaseToken: lease.token }),
      )

      const result = await server._registeredTools.download.handler({
        page: 1,
        ref: 'e12',
      })
      const text = result.content.map((item) => item.text ?? '').join('\n')
      const savedPath = text.match(/to: (.+report\.csv)/)?.[1]
      expect(savedPath).toBeTruthy()
      expect(outputFileAccess.paths.has(savedPath ?? '')).toBe(true)

      const filesystemRead = createReadTool(undefined, {
        allowedOutputPaths: outputFileAccess.paths,
      }) as unknown as {
        execute(input: { path: string }): Promise<{ text: string }>
      }
      const readback = await filesystemRead.execute({ path: savedPath ?? '' })
      expect(readback.text).toContain('neo,1')
    } finally {
      if (previousBrowserosDir === undefined) {
        delete process.env.BROWSEROS_DIR
      } else {
        process.env.BROWSEROS_DIR = previousBrowserosDir
      }
      rmSync(browserosDir, { recursive: true, force: true })
    }
  })
})

class TestConversationRuns {
  readonly associated: Array<{
    conversationId: string
    runId: string
    tabIds: number[]
  }> = []
  private readonly active = new Map<string, ActiveConversationRun>()

  activeRun(conversationId: string): ActiveConversationRun | undefined {
    return this.active.get(conversationId)
  }

  start(conversationId: string, runId: string): ActiveConversationRun {
    const abortController = new AbortController()
    let run!: ActiveConversationRun
    run = {
      conversationId,
      runId,
      panelsVisible: true,
      tabGroup: { title: 'browseros/test', colorKey: 'browseros' },
      signal: abortController.signal,
      associateTabs: (tabIds) => {
        if (this.active.get(conversationId) !== run) return false
        this.associated.push({ conversationId, runId, tabIds: [...tabIds] })
        return true
      },
    }
    this.active.set(conversationId, run)
    return run
  }

  finish(conversationId: string): void {
    this.active.delete(conversationId)
  }
}

function moduleFixture(
  options: { session?: BrowserSession; klavis?: KlavisService } = {},
) {
  const runs = new TestConversationRuns()
  const addCreatedPages = mock(() => {})
  const browserMcp = new BrowserMcpModule({
    version: 'test',
    browserSession: options.session ?? browserSession(),
    conversationRuns: runs,
    klavis: options.klavis,
    tabGroups: { addCreatedPages },
  })
  return { addCreatedPages, browserMcp, runs }
}
