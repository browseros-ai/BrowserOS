/**
 * tabs (6), tab_groups (6) and windows (5) cases. Page/window handles
 * these cases open are cleaned up per case when that is part of the
 * behavior under test; group effects are observed through the tools
 * themselves (`tab_groups list` is the read-back).
 */

import type { ContractCase } from './cases'
import { expectError, expectOk, parsePageId, waitUntil } from './helpers'
import { textOf } from './mcp-client'

/**
 * `tab_groups` responses render as `grouped into [<id>] "<title>" …`;
 * the id is a string in the contract schema (never pass a number).
 */
function parseGroupId(text: string): string {
  const match = text.match(/\[([^\]]+)\]/)
  if (!match) throw new Error(`no group id in: ${text.slice(0, 200)}`)
  return match[1]
}

export const tabsCases: ContractCase[] = [
  {
    name: 'tabs: new foreground page opens with auto-context',
    smoke: true,
    async run(ctx) {
      const result = await ctx.mcp.callTool('tabs', {
        action: 'new',
        url: ctx.fixture('/links.html'),
        background: false,
      })
      const text = expectOk(result, 'tabs new')
      const page = parsePageId(result)
      if (
        !text.includes(`opened page ${page}`) ||
        !text.includes(`[Page ${page} snapshot]`)
      ) {
        throw new Error(`tabs new omitted Rust auto-context: ${text}`)
      }
    },
  },
  {
    name: 'tabs: list shows own pages with urls',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = expectOk(await ctx.mcp.callTool('tabs', { action: 'list' }))
      if (!text.includes(`${page}`) || !text.includes('/links.html')) {
        throw new Error(`tabs list is missing page ${page}: ${text}`)
      }
    },
  },
  {
    name: 'tabs: active reports the focused page',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      let text = ''
      await waitUntil(async () => {
        text = expectOk(
          await ctx.mcp.callTool('tabs', { action: 'active' }),
          'tabs active',
        )
        return text.includes('/form.html') || text.includes(`${page}`)
      }, 'active tab to report the focused form page')
    },
  },
  {
    name: 'tabs: new background hidden page does not become active',
    async run(ctx) {
      const focused = await ctx.openPage(ctx.fixture('/form.html'))
      await waitUntil(async () => {
        const active = expectOk(
          await ctx.mcp.callTool('tabs', { action: 'active' }),
        )
        return active.includes(`[${focused}]`)
      }, 'the foreground page to receive focus')
      const result = await ctx.mcp.callTool('tabs', {
        action: 'new',
        url: ctx.fixture('/links.html'),
        background: true,
        hidden: true,
      })
      expectOk(result, 'tabs new background+hidden')
      const opened = parsePageId(result)
      const active = expectOk(
        await ctx.mcp.callTool('tabs', { action: 'active' }),
      )
      if (active.includes(`[${opened}]`)) {
        throw new Error(`hidden background page became active: ${active}`)
      }
      // Track for cleanup — openPage was bypassed to control background/hidden.
      await ctx.mcp.callTool('tabs', { action: 'close', page: opened })
      await ctx.mcp.callTool('tabs', { action: 'close', page: focused })
    },
  },
  {
    name: 'tabs: close own page removes it from the list',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      expectOk(
        await ctx.mcp.callTool('tabs', { action: 'close', page }),
        'tabs close',
      )
      await waitUntil(async () => {
        const text = textOf(await ctx.mcp.callTool('tabs', { action: 'list' }))
        return !text.includes(`[${page}]`) && !text.includes(`page ${page}`)
      }, 'closed page to leave the tab list')
    },
  },
  {
    name: 'tabs: closing a nonexistent page errors',
    async run(ctx) {
      expectError(
        await ctx.mcp.callTool('tabs', { action: 'close', page: 99_999 }),
        'tabs close on missing page',
      )
    },
  },

  // tab_groups ------------------------------------------------------------
  {
    name: 'tab_groups: tabs new auto-creates the session group',
    async run(ctx) {
      await ctx.openPage(ctx.fixture('/links.html'))
      // Group effects run async after the response; read back until visible.
      let groups = ''
      await waitUntil(async () => {
        groups = expectOk(
          await ctx.mcp.callTool('tab_groups', { action: 'list' }),
        )
        return groups.includes('claw/')
      }, 'the convo tab group to appear')
    },
  },
  {
    name: 'tab_groups: create groups real pages',
    async run(ctx) {
      const pageA = await ctx.openPage(ctx.fixture('/links.html'))
      const pageB = await ctx.openPage(ctx.fixture('/form.html'))
      const created = expectOk(
        await ctx.mcp.callTool('tab_groups', {
          action: 'create',
          pages: [pageA, pageB],
          title: 'contract-group',
          color: 'cyan',
        }),
        'tab_groups create',
      )
      const groupId = parseGroupId(created)
      const list = expectOk(
        await ctx.mcp.callTool('tab_groups', { action: 'list' }),
      )
      if (!list.includes('contract-group')) {
        throw new Error(`created group missing from list: ${list}`)
      }
      if (!list.includes(groupId)) {
        throw new Error(
          `created group id ${groupId} missing from list: ${list}`,
        )
      }
    },
  },
  {
    name: 'tab_groups: update retitles and recolors',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const created = expectOk(
        await ctx.mcp.callTool('tab_groups', {
          action: 'create',
          pages: [page],
          title: 'before-update',
          color: 'yellow',
        }),
      )
      const groupId = parseGroupId(created)
      expectOk(
        await ctx.mcp.callTool('tab_groups', {
          action: 'update',
          groupId,
          title: 'after-update',
          color: 'green',
          collapsed: true,
        }),
        'tab_groups update',
      )
      let list = ''
      await waitUntil(async () => {
        list = expectOk(
          await ctx.mcp.callTool('tab_groups', { action: 'list' }),
        )
        return list.includes('after-update') && !list.includes('before-update')
      }, 'group update to be visible')
    },
  },
  {
    name: 'tab_groups: ungroup releases pages without closing them',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const created = expectOk(
        await ctx.mcp.callTool('tab_groups', {
          action: 'create',
          pages: [page],
          title: 'to-ungroup',
        }),
      )
      parseGroupId(created)
      expectOk(
        await ctx.mcp.callTool('tab_groups', {
          action: 'ungroup',
          pages: [page],
        }),
        'tab_groups ungroup',
      )
      await waitUntil(async () => {
        const list = expectOk(
          await ctx.mcp.callTool('tab_groups', { action: 'list' }),
        )
        return !list.includes('to-ungroup')
      }, 'ungrouped group to disappear')
      const tabs = expectOk(await ctx.mcp.callTool('tabs', { action: 'list' }))
      if (!tabs.includes('/links.html')) {
        throw new Error('ungroup closed the page it should have released')
      }
    },
  },
  {
    name: 'tab_groups: close closes the group and its pages',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const created = expectOk(
        await ctx.mcp.callTool('tab_groups', {
          action: 'create',
          pages: [page],
          title: 'to-close',
        }),
      )
      const groupId = parseGroupId(created)
      expectOk(
        await ctx.mcp.callTool('tab_groups', { action: 'close', groupId }),
        'tab_groups close',
      )
      await waitUntil(async () => {
        const tabs = textOf(await ctx.mcp.callTool('tabs', { action: 'list' }))
        return !tabs.includes(`[${page}]`) && !tabs.includes(`page ${page}`)
      }, 'group close to close its pages')
    },
  },
  {
    name: 'tab_groups: list reflects only live groups',
    async run(ctx) {
      const text = expectOk(
        await ctx.mcp.callTool('tab_groups', { action: 'list' }),
        'tab_groups list',
      )
      if (text.includes('to-close') || text.includes('after-update')) {
        throw new Error(`closed groups remained in the list: ${text}`)
      }
    },
  },

  // windows ---------------------------------------------------------------
  {
    name: 'windows: list shows at least one window',
    async run(ctx) {
      const text = expectOk(
        await ctx.mcp.callTool('windows', { action: 'list' }),
        'windows list',
      )
      if (!/\d/.test(text)) {
        throw new Error(`windows list did not contain a window id: ${text}`)
      }
    },
  },
  {
    name: 'windows: create opens a hidden window',
    async run(ctx) {
      const created = expectOk(
        await ctx.mcp.callTool('windows', { action: 'create', hidden: true }),
        'windows create',
      )
      const windowId = Number(created.match(/\b(\d+)\b/)?.[1])
      if (!Number.isFinite(windowId)) {
        throw new Error(`windows create did not return an id: ${created}`)
      }
      expectOk(
        await ctx.mcp.callTool('windows', { action: 'close', windowId }),
        'windows close hidden',
      )
    },
  },
  {
    name: 'windows: activate focuses the target window',
    async run(ctx) {
      const created = expectOk(
        await ctx.mcp.callTool('windows', { action: 'create' }),
      )
      const windowId = Number(created.match(/\b(\d+)\b/)?.[1])
      const activated = await ctx.mcp.callTool('windows', {
        action: 'activate',
        windowId,
      })
      expectOk(activated, 'windows activate')
      await ctx.mcp.callTool('windows', { action: 'close', windowId })
    },
  },
  {
    name: 'windows: set_visibility toggles a window',
    async run(ctx) {
      const created = expectOk(
        await ctx.mcp.callTool('windows', { action: 'create' }),
      )
      let windowId = Number(created.match(/\b(\d+)\b/)?.[1])
      const hidden = await ctx.mcp.callTool('windows', {
        action: 'set_visibility',
        windowId,
        visible: false,
      })
      expectOk(hidden, 'windows hide')
      // Some browser versions recreate a window while changing its visibility.
      const newId = textOf(hidden).match(/new window id (\d+)/)?.[1]
      if (newId) windowId = Number(newId)
      const shown = await ctx.mcp.callTool('windows', {
        action: 'set_visibility',
        windowId,
        visible: true,
      })
      expectOk(shown, 'windows show')
      const shownId = textOf(shown).match(/new window id (\d+)/)?.[1]
      if (shownId) windowId = Number(shownId)
      expectOk(
        await ctx.mcp.callTool('windows', { action: 'close', windowId }),
        'windows close after visibility changes',
      )
    },
  },
  {
    name: 'windows: close removes the window',
    async run(ctx) {
      const created = expectOk(
        await ctx.mcp.callTool('windows', { action: 'create' }),
      )
      const windowId = Number(created.match(/\b(\d+)\b/)?.[1])
      expectOk(
        await ctx.mcp.callTool('windows', { action: 'close', windowId }),
        'windows close',
      )
      await waitUntil(async () => {
        const list = expectOk(
          await ctx.mcp.callTool('windows', { action: 'list' }),
        )
        return !list.includes(`${windowId}`)
      }, 'closed window to leave the list')
    },
  },
]
