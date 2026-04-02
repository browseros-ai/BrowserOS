import { describe, it } from 'bun:test'
import assert from 'node:assert'
import {
  close_page,
  get_active_page,
  list_pages,
  move_page,
  navigate_page,
  new_hidden_page,
  new_page,
  show_page,
  wait_for,
} from '../../src/tools/navigation'
import { close_window, create_window } from '../../src/tools/windows'
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

describe('navigation tools', () => {
  it('hidden-page tool descriptions do not claim screenshots are unsupported', () => {
    assert.ok(
      !new_hidden_page.description.includes('take_screenshot is not supported'),
    )
    assert.ok(!show_page.description.includes('for screenshots'))
  })

  it('list_pages returns at least one page', async () => {
    await withBrowser(async ({ execute }) => {
      const result = await execute(list_pages, {})
      assert.ok(!result.isError, textOf(result))
      const data = structuredOf<{
        pages: Array<{ pageId: number }>
        count: number
      }>(result)
      assert.ok(data.count > 0, 'Expected non-empty page list')
      assert.ok(data.pages[0]?.pageId !== undefined, 'Expected page IDs')
    })
  }, 60_000)

  it('get_active_page returns the focused page', async () => {
    await withBrowser(async ({ execute }) => {
      const result = await execute(get_active_page, {})
      assert.ok(!result.isError, textOf(result))
      const data = structuredOf<{ page: { pageId: number; tabId: number } }>(
        result,
      )
      assert.ok(data.page.pageId > 0)
      assert.ok(data.page.tabId > 0)
    })
  }, 60_000)

  it('new_page opens a tab and close_page removes it', async () => {
    await withBrowser(async ({ execute }) => {
      const beforeResult = await execute(list_pages, {})
      const beforeCount = structuredOf<{ count: number }>(beforeResult).count

      const newResult = await execute(new_page, { url: 'about:blank' })
      assert.ok(!newResult.isError, textOf(newResult))
      const pageId = structuredOf<{ pageId: number }>(newResult).pageId

      const afterResult = await execute(list_pages, {})
      const afterCount = structuredOf<{ count: number }>(afterResult).count
      assert.ok(afterCount > beforeCount, 'Expected more pages after new_page')

      const closeResult = await execute(close_page, { page: pageId })
      assert.ok(!closeResult.isError, textOf(closeResult))
      assert.ok(textOf(closeResult).includes(`Closed page ${pageId}`))
    })
  }, 60_000)

  it('navigate_page navigates to a URL', async () => {
    await withBrowser(async ({ execute }) => {
      const newResult = await execute(new_page, { url: 'about:blank' })
      const pageId = structuredOf<{ pageId: number }>(newResult).pageId

      const navResult = await execute(navigate_page, {
        page: pageId,
        action: 'url',
        url: 'https://example.com',
      })
      assert.ok(!navResult.isError, textOf(navResult))
      assert.ok(textOf(navResult).includes('Navigated to'))
      const data = structuredOf<{ action: string; page: number }>(navResult)
      assert.strictEqual(data.action, 'url')
      assert.strictEqual(data.page, pageId)

      await execute(close_page, { page: pageId })
    })
  }, 60_000)

  it('wait_for finds text on page', async () => {
    await withBrowser(async ({ execute }) => {
      const newResult = await execute(new_page, { url: 'https://example.com' })
      const pageId = structuredOf<{ pageId: number }>(newResult).pageId

      const waitResult = await execute(wait_for, {
        page: pageId,
        text: 'Example Domain',
        timeout: 10_000,
      })
      assert.ok(!waitResult.isError, textOf(waitResult))
      assert.ok(textOf(waitResult).includes('Found'))
      const data = structuredOf<{ found: boolean; page: number }>(waitResult)
      assert.strictEqual(data.found, true)
      assert.strictEqual(data.page, pageId)

      await execute(close_page, { page: pageId })
    })
  }, 60_000)

  it('wait_for times out for missing text', async () => {
    await withBrowser(async ({ execute }) => {
      const newResult = await execute(new_page, { url: 'about:blank' })
      const pageId = structuredOf<{ pageId: number }>(newResult).pageId

      const waitResult = await execute(wait_for, {
        page: pageId,
        text: 'this-text-does-not-exist-anywhere',
        timeout: 2_000,
      })
      assert.ok(waitResult.isError, 'Expected timeout error')
      assert.ok(textOf(waitResult).includes('Timed out'))

      await execute(close_page, { page: pageId })
    })
  }, 60_000)

  it('new_hidden_page opens a hidden tab', async () => {
    await withBrowser(async ({ execute }) => {
      const result = await execute(new_hidden_page, {
        url: 'about:blank',
      })
      assert.ok(!result.isError, textOf(result))
      const data = structuredOf<{ pageId: number; hidden: boolean }>(result)
      assert.strictEqual(data.hidden, true)
      const pageId = data.pageId
      await execute(close_page, { page: pageId })
    })
  }, 60_000)

  it('show_page restores a hidden page to visible', async () => {
    await withBrowser(async ({ execute }) => {
      const hiddenResult = await execute(new_hidden_page, {
        url: 'about:blank',
      })
      const pageId = structuredOf<{ pageId: number }>(hiddenResult).pageId

      const showResult = await execute(show_page, { page: pageId })
      assert.ok(!showResult.isError, textOf(showResult))
      assert.ok(textOf(showResult).includes('now visible'))
      const data = structuredOf<{
        page: { pageId: number; isHidden: boolean }
      }>(showResult)
      assert.strictEqual(data.page.pageId, pageId)
      assert.strictEqual(data.page.isHidden, false)

      await execute(close_page, { page: pageId })
    })
  }, 60_000)

  it('show_page errors on an already-visible page', async () => {
    await withBrowser(async ({ execute }) => {
      const newResult = await execute(new_page, { url: 'about:blank' })
      const pageId = structuredOf<{ pageId: number }>(newResult).pageId

      const showResult = await execute(show_page, { page: pageId })
      assert.ok(showResult.isError, 'Expected error for visible page')

      await execute(close_page, { page: pageId })
    })
  }, 60_000)

  it('move_page moves a tab to a different window', async () => {
    await withBrowser(async ({ execute }) => {
      const newResult = await execute(new_page, { url: 'about:blank' })
      const pageId = structuredOf<{ pageId: number }>(newResult).pageId

      const winResult = await execute(create_window, {})
      const windowId = structuredOf<{ window: { windowId: number } }>(winResult)
        .window.windowId

      const moveResult = await execute(move_page, {
        page: pageId,
        windowId,
      })
      assert.ok(!moveResult.isError, textOf(moveResult))
      const moveData = structuredOf<{
        page: { pageId: number; windowId?: number }
      }>(moveResult)
      assert.strictEqual(moveData.page.pageId, pageId)
      assert.strictEqual(moveData.page.windowId, windowId)

      await execute(close_page, { page: pageId })
      await execute(close_window, { windowId })
    })
  }, 60_000)
})
