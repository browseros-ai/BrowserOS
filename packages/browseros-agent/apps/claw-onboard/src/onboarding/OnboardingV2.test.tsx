import { afterEach, describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { BrowserOSOnboardingBridge } from './browseros-onboarding-bridge'
import {
  finishBrowserOSOnboarding,
  importPhaseFor,
  OnboardingV2,
  openBrowserOsMcpPage,
} from './OnboardingV2'

const originalWindow = globalThis.window

function renderApp(): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <OnboardingV2 />
    </MemoryRouter>,
  )
}

function installAssignableWindow(search: string) {
  let assignedUrl: string | null = null
  const storage = new Map<string, string>()
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      location: {
        search,
        assign(url: string) {
          assignedUrl = url
        },
      },
      sessionStorage: {
        getItem(key: string) {
          return storage.get(key) ?? null
        },
        setItem(key: string, value: string) {
          storage.set(key, value)
        },
      },
    },
  })
  return () => assignedUrl
}

function stubBridge(isMock: boolean) {
  let completeCount = 0
  const bridge: BrowserOSOnboardingBridge = {
    isMock,
    complete() {
      completeCount += 1
    },
    pageReady() {
      throw new Error('unexpected pageReady call')
    },
    refreshSources() {
      throw new Error('unexpected refreshSources call')
    },
    registerReceiver() {
      throw new Error('unexpected registerReceiver call')
    },
    startImport() {
      throw new Error('unexpected startImport call')
    },
  }

  return {
    bridge,
    getCompleteCount: () => completeCount,
  }
}

afterEach(() => {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: originalWindow,
  })
})

describe('OnboardingV2 shell', () => {
  it('lands on step 0 with the welcome heading and primary CTA', () => {
    const html = renderApp()
    expect(html).toContain('The browser your agents')
    expect(html).toContain('drive')
    expect(html).toContain('Start setup')
  })

  it('renders the visual rail with the v2 quote and three feature blocks', () => {
    const html = renderApp()
    expect(html).toContain('BrowserClaw')
    expect(html).toContain('Let your AI')
    expect(html).toContain('Signed in as you.')
    expect(html).toContain('Watch every step.')
    expect(html).toContain('Yours to keep.')
  })

  it('renders a full-page main landmark without the fake macOS window chrome', () => {
    const html = renderApp()
    expect(html).toContain('<main')
    expect(html).not.toContain('role="dialog"')
    expect(html).not.toContain('Welcome to BrowserClaw')
    expect(html).not.toContain('#FF5F57')
  })

  it('renders three step dots', () => {
    const html = renderApp()
    const matches = html.match(/data-step-dot="true"/g) ?? []
    expect(html).toContain('aria-label="Onboarding progress"')
    expect(matches.length).toBe(3)
  })

  it('opens BrowserClaw MCP page when onboarding completes', () => {
    const getAssignedUrl = installAssignableWindow(
      '?apiUrl=http%3A%2F%2F127.0.0.1%3A9234',
    )

    openBrowserOsMcpPage()

    expect(getAssignedUrl()).toBe('chrome://newtab/#/mcp')
  })

  it('does not navigate after completing through the real Chromium bridge', () => {
    const getAssignedUrl = installAssignableWindow('')
    const { bridge, getCompleteCount } = stubBridge(false)

    finishBrowserOSOnboarding(bridge)

    expect(getCompleteCount()).toBe(1)
    expect(getAssignedUrl()).toBeNull()
  })

  it('keeps navigating after completion in mock standalone onboarding', () => {
    const getAssignedUrl = installAssignableWindow('')
    const { bridge, getCompleteCount } = stubBridge(true)

    finishBrowserOSOnboarding(bridge)

    expect(getCompleteCount()).toBe(1)
    expect(getAssignedUrl()).toBe('chrome://newtab/#/mcp')
  })

  it('does not treat failed or completed Chromium states as import success', () => {
    expect(importPhaseFor('failed')).toBe('failed')
    expect(importPhaseFor('completed')).toBe('picker')
    expect(importPhaseFor('idle')).toBe('picker')
  })
})
