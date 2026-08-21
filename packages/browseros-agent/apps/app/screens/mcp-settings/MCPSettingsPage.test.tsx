import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { createElement, type FC, type ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
// Relative on purpose — see the note in BrowserClawMcpBanner.test.tsx. Nothing
// here asserts on the value, but forwarding the real one keeps every test in this
// folder honest about where the constant comes from.
import { BROWSERCLAW_MCP_BANNER_CLICKED_EVENT } from '../../lib/constants/analyticsEvents'

// The three sibling sections are stubbed so this exercises the page's own
// composition; BrowserClawMcpBanner is deliberately left real, because the
// banner being mounted at all is the thing under test.
mock.module('./MCPServerHeader', () => ({
  MCPServerHeader: () => createElement('div', null, 'server-header'),
}))

mock.module('./IntegrationsSection', () => ({
  IntegrationsSection: () => createElement('div', null, 'integrations'),
}))

mock.module('./MCPToolsSection', () => ({
  MCPToolsSection: () => createElement('div', null, 'tools'),
}))

mock.module('@/assets/browserclaw_logo.png', () => ({
  default: 'logo.png',
}))

mock.module('@/components/ui/button', () => ({
  Button: ({ children }: { children?: ReactNode }) =>
    createElement('button', { type: 'button' }, children),
}))

mock.module('@/lib/metrics/track', () => ({
  track: () => {},
}))

mock.module('@/lib/sentry/sentry', () => ({
  sentry: { captureException: () => {} },
}))

mock.module('@/lib/constants/analyticsEvents', () => ({
  BROWSERCLAW_MCP_BANNER_CLICKED_EVENT,
}))

mock.module('@/lib/browseros/helpers', () => ({
  getMcpServerUrl: async () => 'http://127.0.0.1:9200/mcp',
}))

let MCPSettingsPage: FC

beforeAll(async () => {
  MCPSettingsPage = (await import('./MCPSettingsPage')).MCPSettingsPage
})

describe('MCPSettingsPage', () => {
  it('mounts the permanent BrowserClaw banner between the server header and integrations', () => {
    const html = renderToStaticMarkup(createElement(MCPSettingsPage))

    expect(html).toContain('For better MCP support, use BrowserOS neo')

    // Pin the stub positions first: a missing stub would return -1 and turn the
    // ordering bounds below into assertions that are trivially true.
    const headerIndex = html.indexOf('server-header')
    const integrationsIndex = html.indexOf('integrations')
    expect(headerIndex).toBeGreaterThan(-1)
    expect(integrationsIndex).toBeGreaterThan(-1)

    const bannerIndex = html.indexOf('For better MCP support')
    expect(bannerIndex).toBeGreaterThan(headerIndex)
    expect(bannerIndex).toBeLessThan(integrationsIndex)
  })
})
