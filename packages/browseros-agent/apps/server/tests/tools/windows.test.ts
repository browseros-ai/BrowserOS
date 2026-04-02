import { describe, it } from 'bun:test'
import assert from 'node:assert'
import {
  activate_window,
  close_window,
  create_hidden_window,
  create_window,
  list_windows,
} from '../../src/tools/windows'
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

describe('window tools', () => {
  it('create_hidden_window description does not claim screenshots are unsupported', () => {
    assert.ok(
      !create_hidden_window.description.includes(
        'take_screenshot is not supported',
      ),
    )
  })

  it('list_windows returns at least one window', async () => {
    await withBrowser(async ({ execute }) => {
      const result = await execute(list_windows, {})
      assert.ok(!result.isError, textOf(result))
      const data = structuredOf<{
        windows: Array<{ windowId: number }>
        count: number
      }>(result)
      assert.ok(data.count > 0, 'Expected at least one window')
      assert.ok(data.windows[0]?.windowId !== undefined, 'Expected window IDs')
    })
  }, 60_000)

  it('create and close a window', async () => {
    await withBrowser(async ({ execute }) => {
      const createResult = await execute(create_window, {})
      assert.ok(!createResult.isError, textOf(createResult))
      const windowId = structuredOf<{ window: { windowId: number } }>(
        createResult,
      ).window.windowId

      const closeResult = await execute(close_window, { windowId })
      assert.ok(!closeResult.isError, textOf(closeResult))
      const closeData = structuredOf<{ action: string; windowId: number }>(
        closeResult,
      )
      assert.strictEqual(closeData.action, 'close_window')
      assert.strictEqual(closeData.windowId, windowId)
    })
  }, 60_000)

  it('activate_window focuses a window', async () => {
    await withBrowser(async ({ execute }) => {
      const listResult = await execute(list_windows, {})
      const listData = structuredOf<{ windows: Array<{ windowId: number }> }>(
        listResult,
      )
      const windowId = listData.windows[0]?.windowId
      assert.ok(windowId !== undefined, 'No window found')

      const activateResult = await execute(activate_window, { windowId })
      assert.ok(!activateResult.isError, textOf(activateResult))
      const activateData = structuredOf<{ action: string; windowId: number }>(
        activateResult,
      )
      assert.strictEqual(activateData.action, 'activate_window')
      assert.strictEqual(activateData.windowId, windowId)
    })
  }, 60_000)

  it('create_hidden_window creates and closes a hidden window', async () => {
    await withBrowser(async ({ execute }) => {
      const createResult = await execute(create_hidden_window, {})
      assert.ok(!createResult.isError, textOf(createResult))
      const windowData = structuredOf<{
        window: { windowId: number; isVisible: boolean }
      }>(createResult)
      assert.strictEqual(windowData.window.isVisible, false)
      const windowId = windowData.window.windowId

      const listResult = await execute(list_windows, {})
      const listData = structuredOf<{ windows: Array<{ windowId: number }> }>(
        listResult,
      )
      assert.ok(
        listData.windows.some((w) => w.windowId === windowId),
        'Hidden window should appear in list',
      )

      const closeResult = await execute(close_window, { windowId })
      assert.ok(!closeResult.isError, textOf(closeResult))
    })
  }, 60_000)
})
