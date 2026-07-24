import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = Omit<ComponentProps<'button'>, 'onClick'> & {
  variant?: string
  size?: string
  onClick?: () => void
}

const trackedEvents: string[] = []
const createdTabs: unknown[] = []
let renderedCtaClick: (() => void) | undefined

mock.module('@/assets/browserclaw_logo.png', () => ({
  default: 'logo.png',
}))

mock.module('@/lib/metrics/track', () => ({
  track: (event: string) => {
    trackedEvents.push(event)
  },
}))

mock.module('@/lib/constants/analyticsEvents', () => ({
  BROWSERCLAW_MCP_BANNER_CLICKED_EVENT:
    'settings.browserclaw_mcp_banner.clicked',
}))

mock.module('@/components/ui/button', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: MockButtonProps) => {
    renderedCtaClick = props.onClick
    return createElement('button', { type: 'button', ...props }, children)
  },
}))

let BrowserClawMcpBanner: FC

beforeAll(async () => {
  BrowserClawMcpBanner = (await import('./BrowserClawMcpBanner'))
    .BrowserClawMcpBanner
})

beforeEach(() => {
  trackedEvents.length = 0
  createdTabs.length = 0
  renderedCtaClick = undefined
  globalThis.chrome = {
    tabs: {
      create: async (options: unknown) => {
        createdTabs.push(options)
      },
    },
  } as unknown as typeof chrome
})

function render(): string {
  return renderToStaticMarkup(createElement(BrowserClawMcpBanner))
}

describe('BrowserClawMcpBanner', () => {
  it('renders the MCP-specific BrowserClaw pitch', () => {
    const html = render()

    expect(html).toContain('For better MCP support, use BrowserClaw')
    expect(html).toContain(
      'A browser built for AI agents — a bigger MCP toolset, your real logins, and session replay',
    )
    expect(html).toContain('browserclaw.ai')
  })

  it('does not repeat the copy of the banners that link here', () => {
    const html = render()

    expect(html).not.toContain('Connect your favorite coding tools')
    expect(html).not.toContain('A new product from the BrowserOS team')
  })

  it('is permanent: renders only the CTA, with no dismiss control', () => {
    const html = render()

    expect(html.match(/<button/g) ?? []).toHaveLength(1)
    expect(html).not.toContain('Dismiss')
  })

  it('opens browserclaw.ai and tracks the click', () => {
    render()

    renderedCtaClick?.()

    expect(trackedEvents).toEqual(['settings.browserclaw_mcp_banner.clicked'])
    expect(createdTabs).toEqual([{ url: 'https://browserclaw.ai' }])
  })
})
