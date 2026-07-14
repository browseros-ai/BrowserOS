import { describe, expect, it } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { WindowInfo } from '@browseros/browser-core/core/windows'
import { executeTool } from './framework'
import { BROWSER_TOOLS } from './registry'

const page = {
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

const windowInfo: WindowInfo = {
  windowId: 7,
  windowType: 'normal',
  bounds: {},
  isActive: true,
  isVisible: true,
  tabCount: 1,
}

function createContractSession(): BrowserSession {
  let downloadBegin:
    | ((params: { guid: string; suggestedFilename: string }) => void)
    | undefined
  let downloadProgress:
    | ((params: { guid: string; state: 'completed' }) => void)
    | undefined

  const pageSession = {
    Runtime: {
      evaluate: async (params: { expression: string }) => ({
        result: {
          value:
            params.expression.includes('.includes(') ||
            params.expression.includes('querySelector(')
              ? true
              : 'page value',
        },
      }),
    },
    Page: {
      getLayoutMetrics: async () => ({
        layoutViewport: {
          pageX: 0,
          pageY: 0,
          clientWidth: 1024,
          clientHeight: 768,
        },
      }),
      printToPDF: async () => ({
        data: Buffer.from('pdf').toString('base64'),
      }),
      setDownloadBehavior: async () => {},
      on: (event: string, callback: (params: never) => void): (() => void) => {
        if (event === 'downloadWillBegin') {
          downloadBegin = callback as typeof downloadBegin
          return () => {
            downloadBegin = undefined
          }
        }
        downloadProgress = callback as typeof downloadProgress
        return () => {
          downloadProgress = undefined
        }
      },
    },
  }

  const input = {
    click: async () => {
      if (downloadBegin && downloadProgress) {
        downloadBegin({ guid: 'download-1', suggestedFilename: 'report.txt' })
        downloadProgress({ guid: 'download-1', state: 'completed' })
      }
    },
    uploadFile: async () => {},
  }

  return {
    pages: {
      list: async () => [page],
      getActive: async () => page,
      newPage: async () => 2,
      close: async () => {},
      refresh: async () => page,
      getInfo: (pageId: number) =>
        pageId === 1 ? page : { ...page, pageId, tabId: pageId + 10 },
      getSession: async () => ({ session: pageSession }),
      resolveTabIds: async (tabIds: number[]) =>
        new Map(tabIds.map((tabId) => [tabId, tabId - 10])),
    },
    nav: () => ({
      goto: async () => {},
      back: async () => {},
      forward: async () => {},
      reload: async () => {},
    }),
    observe: () => ({
      snapshot: async () => ({ text: 'button "Save" [ref=e1]' }),
      diff: async () => ({
        changed: true,
        text: '+ button "Saved" [ref=e1]',
        added: 1,
        removed: 0,
        afterUrl: page.url,
      }),
    }),
    input: () => input,
    screenshot: async () => ({
      data: Buffer.from('image').toString('base64'),
      mimeType: 'image/jpeg',
      annotations: [],
    }),
    windows: {
      list: async () => [windowInfo],
      create: async () => windowInfo,
      close: async () => {},
      activate: async () => {},
      setVisibility: async () => ({
        previousWindowId: 7,
        newWindowId: 8,
        replaced: true,
        window: { ...windowInfo, windowId: 8, isVisible: false },
      }),
    },
    cdp: async (method: string) => {
      if (method === 'Browser.getTabGroups') {
        return {
          groups: [
            {
              groupId: 'group-1',
              windowId: 7,
              title: 'Work',
              color: 'blue',
              collapsed: false,
              tabIds: [11],
            },
          ],
        }
      }
      if (
        method === 'Browser.createTabGroup' ||
        method === 'Browser.addTabsToGroup' ||
        method === 'Browser.updateTabGroup'
      ) {
        return {
          group: {
            groupId: 'group-1',
            windowId: 7,
            title: 'Work',
            color: 'blue',
            collapsed: false,
            tabIds: [11],
          },
        }
      }
      return {}
    },
  } as unknown as BrowserSession
}

function structuredKeys(structuredContent: unknown): string[] {
  if (
    typeof structuredContent !== 'object' ||
    structuredContent === null ||
    Array.isArray(structuredContent)
  ) {
    throw new Error('expected a structured object')
  }
  return Object.keys(structuredContent).sort()
}

describe('browser tool structured contract', () => {
  it('pins the exact structured keys emitted by every browser tool', async () => {
    const previous = process.env.BROWSEROS_DIR
    const browserosDir = mkdtempSync(join(tmpdir(), 'structured-contract-'))
    process.env.BROWSEROS_DIR = browserosDir
    try {
      const session = createContractSession()
      const byName = new Map(BROWSER_TOOLS.map((tool) => [tool.name, tool]))
      const call = async (name: string, args: Record<string, unknown>) => {
        const tool = byName.get(name)
        if (!tool) throw new Error(`missing browser tool: ${name}`)
        const result = await executeTool(tool, args, { session })
        expect(result.isError, `${name} should succeed`).toBeFalsy()
        return structuredKeys(result.structuredContent)
      }

      const actual = {
        'tabs.list': await call('tabs', { action: 'list' }),
        'tabs.active': await call('tabs', { action: 'active' }),
        'tabs.new': await call('tabs', { action: 'new' }),
        'tabs.close': await call('tabs', { action: 'close', page: 1 }),
        'tab_groups.list': await call('tab_groups', { action: 'list' }),
        'tab_groups.create': await call('tab_groups', {
          action: 'create',
          pages: [1],
        }),
        'tab_groups.update': await call('tab_groups', {
          action: 'update',
          groupId: 'group-1',
          title: 'Updated',
        }),
        'tab_groups.ungroup': await call('tab_groups', {
          action: 'ungroup',
          pages: [1],
        }),
        'tab_groups.close': await call('tab_groups', {
          action: 'close',
          groupId: 'group-1',
        }),
        navigate: await call('navigate', {
          page: 1,
          action: 'url',
          url: page.url,
        }),
        snapshot: await call('snapshot', { page: 1 }),
        diff: await call('diff', { page: 1 }),
        act: await call('act', { page: 1, kind: 'click', ref: 'e1' }),
        download: await call('download', { page: 1, ref: 'e1' }),
        upload: await call('upload', {
          page: 1,
          ref: 'e2',
          file: '/tmp/upload.txt',
        }),
        read: await call('read', { page: 1, format: 'text' }),
        grep: await call('grep', {
          page: 1,
          pattern: 'save',
          over: 'ax',
        }),
        screenshot: await call('screenshot', { page: 1 }),
        pdf: await call('pdf', { page: 1 }),
        'wait.time': await call('wait', {
          page: 1,
          for: 'time',
          value: 0,
        }),
        'wait.selector': await call('wait', {
          page: 1,
          for: 'selector',
          value: '#ready',
        }),
        'windows.list': await call('windows', { action: 'list' }),
        'windows.create': await call('windows', { action: 'create' }),
        'windows.close': await call('windows', {
          action: 'close',
          windowId: 7,
        }),
        'windows.activate': await call('windows', {
          action: 'activate',
          windowId: 7,
        }),
        'windows.set_visibility': await call('windows', {
          action: 'set_visibility',
          windowId: 7,
          visible: false,
        }),
        evaluate: await call('evaluate', {
          page: 1,
          code: 'return document.title',
        }),
        run: await call('run', { code: 'return { answer: 42 }' }),
      }

      expect(actual).toEqual({
        'tabs.list': ['pages'],
        'tabs.active': ['action', 'page'],
        'tabs.new': ['page'],
        'tabs.close': ['page'],
        'tab_groups.list': ['count', 'groups'],
        'tab_groups.create': ['group'],
        'tab_groups.update': ['group'],
        'tab_groups.ungroup': ['count', 'pageIds'],
        'tab_groups.close': ['groupId'],
        navigate: ['page', 'url'],
        snapshot: ['contentLength', 'page', 'tokenEstimate', 'writtenToFile'],
        diff: ['added', 'changed', 'removed'],
        act: ['changed', 'kind'],
        download: ['filename', 'page', 'path', 'ref'],
        upload: ['files', 'page', 'ref', 'uploaded'],
        read: ['contentLength', 'format', 'page', 'writtenToFile'],
        grep: ['count', 'matches', 'over', 'page'],
        screenshot: ['bytes', 'format', 'page'],
        pdf: ['bytes', 'page', 'path'],
        'wait.time': ['matched', 'waitedMs'],
        'wait.selector': ['matched'],
        'windows.list': ['action', 'count', 'windows'],
        'windows.create': ['action', 'window'],
        'windows.close': ['action', 'windowId'],
        'windows.activate': ['action', 'windowId'],
        'windows.set_visibility': [
          'action',
          'newWindowId',
          'previousWindowId',
          'replaced',
          'window',
        ],
        evaluate: ['page', 'value'],
        run: ['logs', 'ok', 'value'],
      })
    } finally {
      if (previous === undefined) {
        delete process.env.BROWSEROS_DIR
      } else {
        process.env.BROWSEROS_DIR = previous
      }
      rmSync(browserosDir, { recursive: true, force: true })
    }
  })
})
