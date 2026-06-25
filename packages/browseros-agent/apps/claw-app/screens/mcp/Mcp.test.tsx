/**
 * Pins the v2 MCP page shape: hero card with one URL + CLI snippet,
 * "Connected agents" board with one row per harness, "N of M
 * connected" install badge, no McpRow / RegenerateUrlDialog /
 * "Add agent" CTA.
 */

import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'

mock.module('@/modules/api/connections.hooks', () => ({
  useBrowserosConnections: Object.assign(
    () => ({
      data: {
        connections: [
          {
            harness: 'Claude Code',
            installed: false,
            agentId: 'claude-code',
            message: '',
          },
          {
            harness: 'Cursor',
            installed: true,
            agentId: 'cursor',
            configPath: '/tmp/cursor.json',
            message: 'Configured in Cursor.',
          },
          {
            harness: 'Codex',
            installed: false,
            agentId: 'codex',
            message: '',
          },
          {
            harness: 'Hermes',
            installed: true,
            agentId: null,
            message: 'Runs inside BrowserOS.',
          },
        ],
      },
      isPending: false,
      isError: false,
    }),
    { getKey: () => ['cockpit', 'connections'] },
  ),
  useConnectBrowseros: () => ({
    isPending: false,
    variables: undefined,
    mutateAsync: async () => ({ installed: true }),
  }),
  useDisconnectBrowseros: () => ({
    isPending: false,
    variables: undefined,
    mutateAsync: async () => ({ installed: false }),
  }),
}))

const { Mcp } = await import('./Mcp')

function renderApp(): string {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Mcp />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('Mcp (v2)', () => {
  it('renders the hero card with the slugless URL and the canonical CLI snippet', () => {
    const html = renderApp()
    expect(html).toContain('MCP')
    expect(html).toContain('1 endpoint')
    expect(html).toContain('/cockpit/mcp')
    expect(html).not.toContain('/cockpit/mcp/claude-code')
    expect(html).toContain('claude mcp add browseros')
    expect(html).toContain('--transport http')
  })

  it('renders the Connected agents header with the right install badge', () => {
    const html = renderApp()
    expect(html).toContain('Connected agents')
    // External connected = 1 (Cursor), external total = 3 (Claude Code,
    // Cursor, Codex). Hermes (internal) is excluded from the badge.
    expect(html).toContain('1 of 3 connected')
  })

  it('renders one row per harness from the fixture and surfaces install state', () => {
    const html = renderApp()
    expect(html).toContain('Claude Code')
    expect(html).toContain('Cursor')
    expect(html).toContain('Codex')
    expect(html).toContain('Hermes')
    expect(html).toContain('Connected')
    expect(html).toContain('Connect')
  })

  it('renders the internal-harness note at the bottom', () => {
    const html = renderApp()
    expect(html).toContain('Hermes and OpenClaw run inside BrowserOS')
  })

  it('does NOT render the legacy McpRow / RegenerateUrlDialog / "Add agent" CTA', () => {
    const html = renderApp()
    expect(html).not.toContain('Add agent')
    expect(html).not.toContain('Regenerate URL')
    expect(html).not.toContain('No endpoints yet')
  })

  it('renders Hermes as a "Built-in" pill, not a Connect button', () => {
    const html = renderApp()
    // The Hermes row carries the Built-in pill literal copy.
    expect(html).toContain('Built-in')
  })
})
