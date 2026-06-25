import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import type {
  RuntimeSidePanelScopeChangedData,
  RuntimeStopAgentData,
  RuntimeTabIdResponse,
} from './runtimeMessages'

const readAgentFile = (path: string) =>
  readFileSync(new URL(`../../../${path}`, import.meta.url), 'utf8')

describe('runtime message protocol', () => {
  it('uses namespaced typed message names instead of legacy raw types', () => {
    const source = readAgentFile('lib/messaging/runtime/runtimeMessages.ts')

    expect(source).toContain("getTabId: 'runtime.getTabId'")
    expect(source).toContain("authSuccess: 'runtime.authSuccess'")
    expect(source).toContain("stopAgent: 'runtime.stopAgent'")
    expect(source).toContain(
      "sidePanelScopeChanged: 'runtime.sidePanelScopeChanged'",
    )
    expect(source).not.toContain("'get-tab-id'")
    expect(source).not.toContain("'AUTH_SUCCESS'")
    expect(source).not.toContain("'stop-agent'")
  })

  it('keeps the vulnerable content scripts off raw runtime message shapes', () => {
    const files = [
      'entrypoints/selection.content.ts',
      'entrypoints/auth.content/index.ts',
      'entrypoints/glow.content/index.ts',
    ]

    for (const file of files) {
      expect(readAgentFile(file)).not.toMatch(
        /(?:chrome|browser)\.runtime\.sendMessage\(\s*\{\s*type\s*:/,
      )
    }
  })

  it('keeps background handling off the legacy raw runtime listener', () => {
    const source = readAgentFile('entrypoints/background/index.ts')

    expect(source).not.toContain('chrome.runtime.onMessage.addListener')
    expect(source).not.toContain("message?.type === 'get-tab-id'")
    expect(source).not.toContain("message?.type === 'AUTH_SUCCESS'")
    expect(source).not.toContain("message?.type === 'stop-agent'")
  })

  it('keeps the stop-agent and tab-id payload contracts explicit', () => {
    const stopAgent = {
      conversationId: 'conversation-1',
    } satisfies RuntimeStopAgentData
    const tabId = { tabId: 123 } satisfies RuntimeTabIdResponse
    const sidePanelScope = {
      perWindow: true,
    } satisfies RuntimeSidePanelScopeChangedData

    expect(stopAgent.conversationId).toBe('conversation-1')
    expect(tabId.tabId).toBe(123)
    expect(sidePanelScope.perWindow).toBe(true)
  })
})
