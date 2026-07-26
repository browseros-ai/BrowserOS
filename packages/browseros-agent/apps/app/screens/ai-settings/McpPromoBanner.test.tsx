import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

mock.module('react-router', () => ({
  useNavigate: () => () => {},
}))

mock.module('@/lib/metrics/track', () => ({
  track: () => {},
}))

mock.module('@/lib/constants/analyticsEvents', () => ({
  MCP_PROMO_BANNER_CLICKED_EVENT: 'settings.mcp_promo_banner.clicked',
}))

mock.module('@/components/ui/button', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: MockButtonProps) =>
    createElement('button', { type: 'button', ...props }, children),
}))

let McpPromoBanner: FC

beforeAll(async () => {
  McpPromoBanner = (await import('./McpPromoBanner')).McpPromoBanner
})

function renderBanner() {
  return renderToStaticMarkup(createElement(McpPromoBanner))
}

describe('McpPromoBanner', () => {
  it('renders the MCP promo without a fixed tool count', () => {
    const html = renderBanner()

    expect(html).toContain('Use BrowserOS with Claude Code')
    expect(html).toContain('Connect your favorite coding tools')
    expect(html).toContain('Set up')
    expect(html).not.toContain('66+ tools')
  })
})
