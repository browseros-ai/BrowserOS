/**
 * The examples agents read are the code under test. Each case supplies live ids,
 * executes the registry snippet unchanged, and checks browser state before cleanup.
 * No recipe may target a window or group outside this case's disposable state.
 */
import { expect } from 'bun:test'
import recipes from '../../../resources/skills/browserclaw/recipes.json'
import type { CaseContext, ContractCase } from './cases'
import { expectError, expectOk, waitUntil } from './helpers'
import { McpSession } from './mcp-client'
import { startRustServer } from './rust-server'

const scenarios: Record<string, { prepare: string; verify: string }> = {
  'list-groups': {
    prepare:
      "groupId = (await neo.cdp('Browser.createTabGroup', { tabIds: [primary.tabId], title: 'recipe list' })).group.groupId;",
    verify:
      'expect(result.groups.some(group => group.groupId === groupId)).toBe(true);',
  },
  'create-group': {
    prepare: '',
    verify:
      "expect(result.group.title).toBe('research'); expect(result.group.tabIds).toEqual([primary.tabId]); expect((await neo.cdp('Browser.getTabGroups')).groups.some(group => group.groupId === result.group.groupId)).toBe(true);",
  },
  'update-group': {
    prepare:
      "groupId = (await neo.cdp('Browser.createTabGroup', { tabIds: [primary.tabId], title: 'before' })).group.groupId;",
    verify:
      "const group = (await neo.cdp('Browser.getTabGroups')).groups.find(group => group.groupId === groupId); expect(group.title).toBe('research notes'); expect(group.color).toBe('blue'); expect(group.collapsed).toBe(false); expect(group.tabIds).toEqual([primary.tabId]);",
  },
  'add-to-group': {
    prepare:
      "groupId = (await neo.cdp('Browser.createTabGroup', { tabIds: [primary.tabId] })).group.groupId;",
    verify:
      "const group = (await neo.cdp('Browser.getTabGroups')).groups.find(group => group.groupId === groupId); expect(group.tabIds.includes(other.tabId)).toBe(true); expect(group.tabIds.includes(primary.tabId)).toBe(true);",
  },
  'remove-from-group': {
    prepare:
      "groupId = (await neo.cdp('Browser.createTabGroup', { tabIds: [primary.tabId, other.tabId] })).group.groupId;",
    verify:
      "const tabs = (await neo.cdp('Browser.getTabs')).tabs; expect(tabs.some(tab => tab.tabId === other.tabId)).toBe(true); expect(tabs.find(tab => tab.tabId === other.tabId).groupId || null).toBe(null); expect((await neo.cdp('Browser.getTabGroups')).groups.find(group => group.groupId === groupId).tabIds).toEqual([primary.tabId]);",
  },
  'close-group': {
    prepare:
      "groupId = (await neo.cdp('Browser.createTabGroup', { tabIds: [primary.tabId] })).group.groupId;",
    verify:
      "expect((await neo.cdp('Browser.getTabGroups')).groups.some(group => group.groupId === groupId)).toBe(false); expect((await neo.cdp('Browser.getTabs')).tabs.some(tab => tab.tabId === primary.tabId)).toBe(false);",
  },
  'list-windows': {
    prepare: '',
    verify:
      "const tab = (await neo.cdp('Browser.getTabs')).tabs.find(tab => tab.tabId === primary.tabId); expect(result.windows.some(window => window.windowId === tab.windowId)).toBe(true);",
  },
  'open-window': {
    prepare: '',
    verify:
      "openedWindowId = result.window.windowId; expect((await neo.cdp('Browser.getWindows')).windows.some(window => window.windowId === openedWindowId)).toBe(true);",
  },
  'close-window': {
    prepare:
      "windowId = (await neo.cdp('Browser.createWindow', {})).window.windowId;",
    verify:
      "expect((await neo.cdp('Browser.getWindows')).windows.some(window => window.windowId === windowId)).toBe(false); windowId = undefined;",
  },
  'list-profiles': {
    prepare:
      "const { profiles } = await neo.cdp('Browser.getProfiles'); const browserProfileId = profiles.find(profile => profile.loaded).browserProfileId;",
    verify:
      'expect(result.profiles.some(profile => profile.browserProfileId === browserProfileId)).toBe(true);',
  },
  'load-profile': {
    prepare:
      "const { profiles } = await neo.cdp('Browser.getProfiles'); const browserProfileId = profiles.find(profile => profile.loaded).browserProfileId;",
    verify:
      'expect(result.profile.browserProfileId).toBe(browserProfileId); expect(result.profile.loaded).toBe(true); expect(typeof result.profile.browserContextId).toBe("string");',
  },
  'recent-history': {
    prepare: '',
    verify:
      'expect(result.entries.some(entry => entry.url === url)).toBe(true); expect(result.entries.length <= 20).toBe(true);',
  },
  'focus-tab': {
    prepare: '',
    verify:
      "expect((await neo.cdp('Browser.getTabs')).tabs.find(tab => tab.tabId === primary.tabId).isActive).toBe(true);",
  },
}

async function withRecipeSession(
  ctx: CaseContext,
  recipeId: string,
  check: (mcp: McpSession) => Promise<void>,
): Promise<void> {
  if (recipeId !== 'list-profiles' && recipeId !== 'load-profile') {
    return check(ctx.mcp)
  }

  // The generic sidecar has no native identity. Enable profile routing here,
  // then probe the actual browser: older fixtures may lack the native methods.
  const server = await startRustServer(ctx.browser.cdpPort, {
    env: { CLAW_NATIVE_BROWSEROS_VERSION: '0.51.1.3' },
  })
  try {
    const mcp = await McpSession.connect(server.baseUrl, 'recipe-profiles')
    try {
      // HTTP health precedes the SDK's browser attachment. Probe the actual run
      // boundary so the first recipe cannot race that asynchronous handoff.
      await waitUntil(async () => {
        const response = await mcp.callTool('run', { code: 'return true' })
        return !response.isError && response.structuredContent?.value === true
      }, 'recipe browser attachment')
      await check(mcp)
    } finally {
      await mcp.close()
    }
  } finally {
    await server.stop()
  }
}

export const recipeCases: ContractCase[] = recipes.map((recipe) => ({
  name: `recipes: ${recipe.title}`,
  async run(ctx) {
    return withRecipeSession(ctx, recipe.id, async (mcp) => {
      if (recipe.id === 'list-profiles' || recipe.id === 'load-profile') {
        const probe = await mcp.callTool('run', {
          code: "return await neo.cdp('Browser.getProfiles')",
        })
        if (probe.isError) {
          expect(expectError(probe)).toContain(
            "script / cdp_error: CDP error: 'Browser.getProfiles' wasn't found",
          )
          // Without discovery there is no saved-profile handle. A sentinel lets
          // the unchanged load recipe reach dispatch; only a missing method is accepted.
          const unsupported = await mcp.callTool('run', {
            code: `const browserProfileId = 'unsupported-profile-probe'; ${recipe.code}`,
          })
          const method =
            recipe.id === 'list-profiles'
              ? 'Browser.getProfiles'
              : 'Browser.loadProfile'
          expect(expectError(unsupported)).toContain(
            `script / cdp_error: CDP error: '${method}' wasn't found`,
          )
          const next = await mcp.callTool('run', { code: 'return true' })
          expectOk(next, 'a run after the unsupported profile method')
          expect(next.structuredContent?.value).toBe(true)
          return
        }
        expectOk(probe, 'profile command probe')
      }
      if (recipe.id === 'open-tab-in-window') {
        const opened = await mcp.callTool('run', {
          code: `
const { window } = await neo.cdp('Browser.createWindow', {});
const windowId = window.windowId;
const url = ${JSON.stringify(ctx.fixture('/form.html'))};
try {
  const result = await (async () => { ${recipe.code} })();
  return { ...result, windowId };
} catch (error) {
  await neo.cdp('Browser.closeWindow', { windowId });
  throw error;
}`,
        })
        expectOk(opened)
        const created = opened.structuredContent?.value
        if (
          !created ||
          typeof created !== 'object' ||
          !('tabId' in created) ||
          typeof created.tabId !== 'number' ||
          !('windowId' in created) ||
          typeof created.windowId !== 'number'
        )
          throw new Error(
            'The window recipe omitted its created tab or window id',
          )
        const { tabId, windowId } = created
        // Raw CDP creation is learned by the next run's page census. Waiting inside
        // the creating run cannot refresh context.pages(), which is a local list.
        const adopted = await mcp.callTool('run', {
          code: `
try {
  const found = context.pages().find(page => page.tabId === ${tabId});
  expect(found).toBeDefined();
  expect(await found.title()).toBeTruthy();
  return ${JSON.stringify(recipe.id)};
} finally {
  await neo.cdp('Browser.closeWindow', { windowId: ${windowId} });
}`,
        })
        expectOk(adopted)
        expect(adopted.structuredContent?.value).toBe(recipe.id)
        return
      }
      const scenario = scenarios[recipe.id]
      if (!scenario)
        throw new Error(`Recipe ${recipe.id} needs a live scenario`)
      const response = await mcp.callTool('run', {
        code: `
const task = await neo.createTask('recipe check');
let windowId, openedWindowId;
try {
  const primary = await task.newPage();
  const other = await task.newPage();
  const url = ${JSON.stringify(ctx.fixture('/form.html'))};
  await primary.goto(url);
  const tabIds = [other.tabId];
  let groupId;
  ${scenario.prepare}
  const result = await (async (page) => { ${recipe.code} })(primary);
  ${scenario.verify}
  return ${JSON.stringify(recipe.id)};
} finally {
  if (openedWindowId) await neo.cdp('Browser.closeWindow', { windowId: openedWindowId });
  if (windowId) await neo.cdp('Browser.closeWindow', { windowId });
  await task.close();
}`,
      })
      expectOk(response)
      expect(response.structuredContent?.value).toBe(recipe.id)
    })
  },
}))
