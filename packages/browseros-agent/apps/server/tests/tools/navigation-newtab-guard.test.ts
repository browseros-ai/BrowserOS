/**
 * New-tab origin navigation guards.
 *
 * When the chat session originates from the new-tab page, navigate_page and
 * close_page must reject attempts to act on the origin tab. These are
 * integration tests that run against a real browser to verify the guards
 * work end-to-end through executeTool.
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'
import type { ToolContext, ToolDefinition } from '../../src/tools/framework'
import { executeTool } from '../../src/tools/framework'
import { close_page, navigate_page, new_page } from '../../src/tools/navigation'
import type { ToolResult } from '../../src/tools/response'
import { withBrowser } from '../__helpers__/with-browser'

function textOf(result: {
  content: { type: string; text?: string }[]
}): string {
  return result.content
    .filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n')
}

function structuredOf<T>(result: { structuredContent?: unknown }): T {
  assert.ok(result.structuredContent, 'Expected structuredContent')
  return result.structuredContent as T
}

describe('new-tab origin navigation guards', () => {
  // Helper: execute a tool with newtab session context
  function executeWithSession(
    ctx: { browser: ToolContext['browser'] },
    tool: ToolDefinition,
    args: unknown,
    session: ToolContext['session'],
  ): Promise<ToolResult> {
    const signal = AbortSignal.timeout(30_000)
    return executeTool(
      tool,
      args,
      {
        browser: ctx.browser,
        directories: { workingDir: process.cwd() },
        session,
      },
      signal,
    )
  }

  // -------------------------------------------------------------------------
  // navigate_page guards
  // -------------------------------------------------------------------------

  it('navigate_page rejects navigation on origin tab in newtab mode', async () => {
    await withBrowser(async ({ browser }) => {
      // Use a new page as the simulated "origin tab"
      const setupResult = await executeTool(
        new_page,
        { url: 'about:blank' },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
      const originPageId = structuredOf<{ pageId: number }>(setupResult).pageId

      const result = await executeWithSession(
        { browser },
        navigate_page,
        { page: originPageId, action: 'url', url: 'https://example.com' },
        { origin: 'newtab', originPageId },
      )

      assert.ok(result.isError, 'Expected navigate_page to be rejected')
      assert.ok(
        textOf(result).includes('Cannot navigate the origin tab'),
        `Expected origin tab error, got: ${textOf(result)}`,
      )

      // Cleanup
      await executeTool(
        close_page,
        { page: originPageId },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
    })
  }, 60_000)

  it('navigate_page allows navigation on non-origin tab in newtab mode', async () => {
    await withBrowser(async ({ browser }) => {
      const originResult = await executeTool(
        new_page,
        { url: 'about:blank' },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
      const originPageId = structuredOf<{ pageId: number }>(originResult).pageId

      // Open a second tab — this is NOT the origin tab
      const otherResult = await executeTool(
        new_page,
        { url: 'about:blank' },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
      const otherPageId = structuredOf<{ pageId: number }>(otherResult).pageId

      const result = await executeWithSession(
        { browser },
        navigate_page,
        { page: otherPageId, action: 'url', url: 'https://example.com' },
        { origin: 'newtab', originPageId },
      )

      assert.ok(
        !result.isError,
        `Expected success, got error: ${textOf(result)}`,
      )
      assert.ok(textOf(result).includes('Navigated to'))

      // Cleanup
      const noSession = { browser, directories: { workingDir: process.cwd() } }
      await executeTool(
        close_page,
        { page: otherPageId },
        noSession,
        AbortSignal.timeout(30_000),
      )
      await executeTool(
        close_page,
        { page: originPageId },
        noSession,
        AbortSignal.timeout(30_000),
      )
    })
  }, 60_000)

  it('navigate_page works normally in sidepanel mode', async () => {
    await withBrowser(async ({ browser }) => {
      const setupResult = await executeTool(
        new_page,
        { url: 'about:blank' },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
      const pageId = structuredOf<{ pageId: number }>(setupResult).pageId

      const result = await executeWithSession(
        { browser },
        navigate_page,
        { page: pageId, action: 'url', url: 'https://example.com' },
        { origin: 'sidepanel', originPageId: pageId },
      )

      assert.ok(
        !result.isError,
        `Expected success, got error: ${textOf(result)}`,
      )
      assert.ok(textOf(result).includes('Navigated to'))

      await executeTool(
        close_page,
        { page: pageId },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
    })
  }, 60_000)

  it('navigate_page works when session is undefined (backwards compat)', async () => {
    await withBrowser(async ({ browser, execute }) => {
      const setupResult = await execute(new_page, { url: 'about:blank' })
      const pageId = structuredOf<{ pageId: number }>(setupResult).pageId

      // execute() from withBrowser passes no session — simulates old clients
      const result = await execute(navigate_page, {
        page: pageId,
        action: 'url',
        url: 'https://example.com',
      })

      assert.ok(
        !result.isError,
        `Expected success, got error: ${textOf(result)}`,
      )

      await execute(close_page, { page: pageId })
    })
  }, 60_000)

  // -------------------------------------------------------------------------
  // close_page guards
  // -------------------------------------------------------------------------

  it('close_page rejects closing origin tab in newtab mode', async () => {
    await withBrowser(async ({ browser }) => {
      const setupResult = await executeTool(
        new_page,
        { url: 'about:blank' },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
      const originPageId = structuredOf<{ pageId: number }>(setupResult).pageId

      const result = await executeWithSession(
        { browser },
        close_page,
        { page: originPageId },
        { origin: 'newtab', originPageId },
      )

      assert.ok(result.isError, 'Expected close_page to be rejected')
      assert.ok(
        textOf(result).includes('Cannot close the origin tab'),
        `Expected origin tab error, got: ${textOf(result)}`,
      )

      // Clean up the page we created (without newtab guard)
      await executeTool(
        close_page,
        { page: originPageId },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
    })
  }, 60_000)

  it('close_page allows closing non-origin tab in newtab mode', async () => {
    await withBrowser(async ({ browser }) => {
      const originResult = await executeTool(
        new_page,
        { url: 'about:blank' },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
      const originPageId = structuredOf<{ pageId: number }>(originResult).pageId

      const otherResult = await executeTool(
        new_page,
        { url: 'about:blank' },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
      const otherPageId = structuredOf<{ pageId: number }>(otherResult).pageId

      const result = await executeWithSession(
        { browser },
        close_page,
        { page: otherPageId },
        { origin: 'newtab', originPageId },
      )

      assert.ok(
        !result.isError,
        `Expected success, got error: ${textOf(result)}`,
      )
      assert.ok(textOf(result).includes(`Closed page ${otherPageId}`))

      // Cleanup origin page
      await executeTool(
        close_page,
        { page: originPageId },
        { browser, directories: { workingDir: process.cwd() } },
        AbortSignal.timeout(30_000),
      )
    })
  }, 60_000)
})
