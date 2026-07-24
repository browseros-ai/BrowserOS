import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

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
  Button: ({ children }: { children?: unknown }) =>
    createElement('button', { type: 'button' }, children as never),
}))

mock.module('@/lib/metrics/track', () => ({
  track: () => {},
}))

mock.module('@/lib/sentry/sentry', () => ({
  sentry: { captureException: () => {} },
}))

mock.module('@/lib/constants/analyticsEvents', () => ({
  BROWSERCLAW_MCP_BANNER_CLICKED_EVENT:
    'settings.browserclaw_mcp_banner.clicked',
}))

mock.module('@/lib/browseros/helpers', () => ({
  getMcpServerUrl: async () => 'http://127.0.0.1:9200/mcp',
}))

mock.module('@/lib/messaging/server/serverMessages', () => ({
  sendServerMessage: async () => ({ tools: [] }),
}))

let MCPSettingsPage: FC

beforeAll(async () => {
  MCPSettingsPage = (await import('./MCPSettingsPage')).MCPSettingsPage
})

describe('MCPSettingsPage', () => {
  it('mounts the permanent BrowserClaw banner between the server header and integrations', () => {
    const html = renderToStaticMarkup(createElement(MCPSettingsPage))

    expect(html).toContain('For better MCP support, use BrowserClaw')

    const bannerIndex = html.indexOf('For better MCP support')
    expect(bannerIndex).toBeGreaterThan(html.indexOf('server-header'))
    expect(bannerIndex).toBeLessThan(html.indexOf('integrations'))
  })
})
