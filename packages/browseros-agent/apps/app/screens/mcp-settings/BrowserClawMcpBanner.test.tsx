import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
// Imported relatively on purpose: `@/…` does not resolve under `bun test`, so the
// component's copy of this module has to be mocked. Pulling the real value in
// through a path the mock does not intercept keeps the assertion honest — mocking
// the constant *and* asserting a literal would pass even if the shipped event
// value were wrong.
import { BROWSERCLAW_MCP_BANNER_CLICKED_EVENT } from '../../lib/constants/analyticsEvents'

type MockButtonProps = Omit<ComponentProps<'button'>, 'onClick'> & {
  variant?: string
  size?: string
  onClick?: () => void | Promise<void>
}

const trackedEvents: string[] = []
const createdTabs: unknown[] = []
const capturedErrors: unknown[] = []
let renderedCtaClick: (() => void | Promise<void>) | undefined
let tabsCreateError: Error | null = null

mock.module('@/assets/browserclaw_logo.png', () => ({
  default: 'logo.png',
}))

mock.module('@/lib/metrics/track', () => ({
  track: (event: string) => {
    trackedEvents.push(event)
  },
}))

mock.module('@/lib/sentry/sentry', () => ({
  sentry: {
    captureException: (error: unknown) => {
      capturedErrors.push(error)
    },
  },
}))

mock.module('@/lib/constants/analyticsEvents', () => ({
  BROWSERCLAW_MCP_BANNER_CLICKED_EVENT,
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
  capturedErrors.length = 0
  renderedCtaClick = undefined
  tabsCreateError = null
  globalThis.chrome = {
    tabs: {
      create: async (options: unknown) => {
        if (tabsCreateError) throw tabsCreateError
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

  it('is permanent: renders only the CTA, with no dismiss control', () => {
    const html = render()

    expect(html.match(/<button/g) ?? []).toHaveLength(1)
    expect(html).not.toContain('Dismiss')
  })

  // Stands alone rather than riding inside the click test below: this is the one
  // assertion that ties the suite to the value analytics actually ships, so it
  // should not disappear along with a behavior test someone reworks later.
  it('pins the shipped event value', () => {
    expect(BROWSERCLAW_MCP_BANNER_CLICKED_EVENT).toBe(
      'settings.browserclaw_mcp_banner.clicked',
    )
  })

  it('opens browserclaw.ai and tracks the click', async () => {
    render()

    await renderedCtaClick?.()

    expect(trackedEvents).toEqual([BROWSERCLAW_MCP_BANNER_CLICKED_EVENT])
    expect(createdTabs).toEqual([{ url: 'https://browserclaw.ai' }])
    expect(capturedErrors).toEqual([])
  })

  it('reports to Sentry when the tab cannot be opened', async () => {
    tabsCreateError = new Error('no window available')
    render()

    await renderedCtaClick?.()

    expect(capturedErrors).toHaveLength(1)
    expect(capturedErrors[0]).toBe(tabsCreateError)
    expect(createdTabs).toEqual([])
  })
})
