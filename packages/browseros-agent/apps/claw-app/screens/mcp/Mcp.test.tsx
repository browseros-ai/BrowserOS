/**
 * Pins the editorial MCP page shape: compressed hero + single
 * endpoint URL strip, inline Connected-agents header with an
 * `N of M connected` mono chip, hairline row list of the 7 supported
 * harnesses.
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
            harness: 'OpenCode',
            installed: false,
            agentId: 'opencode',
            message: '',
          },
          {
            harness: 'Antigravity',
            installed: false,
            agentId: 'antigravity',
            message: '',
          },
          {
            harness: 'VS Code',
            installed: false,
            agentId: 'vscode',
            message: '',
          },
          {
            harness: 'Zed',
            installed: false,
            agentId: 'zed',
            message: '',
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
const { HeroCard } = await import('./HeroCard')

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

describe('Mcp (editorial)', () => {
  it('renders the editorial hero without exposing the fallback endpoint before pref resolution', () => {
    const html = renderApp()
    expect(html).toContain('MCP')
    expect(html).toContain('every')
    expect(html).toContain('harness.')
    expect(html).not.toContain('http://127.0.0.1:9200/mcp')
    expect(html).not.toContain('copy')
  })

  it('renders the endpoint copy strip once the resolved URL is available', () => {
    const html = renderToStaticMarkup(
      <HeroCard url="http://127.0.0.1:9512/mcp" />,
    )

    expect(html).toContain('http://127.0.0.1:9512/mcp')
    expect(html).not.toContain('/mcp/claude-code')
    expect(html).not.toContain('/cockpit')
    expect(html).toContain('copy')
  })

  it('does NOT render the removed CLI snippet block', () => {
    const html = renderApp()
    expect(html).not.toContain('CLI SNIPPET')
    // Guard against any CLI snippet resurfacing regardless of the
    // registered server name (`browseros` legacy or `BrowserClaw`
    // post-rename).
    expect(html).not.toContain('claude mcp add')
    expect(html).not.toContain('--transport http')
  })

  it('renders the Connected-agents header with the count chip', () => {
    const html = renderApp()
    expect(html).toContain('Connected agents')
    // 7 supported harnesses, 1 connected (Cursor).
    expect(html).toContain('1 of 7 connected')
  })

  it('renders one row per supported harness', () => {
    const html = renderApp()
    expect(html).toContain('Claude Code')
    expect(html).toContain('Cursor')
    expect(html).toContain('Codex')
    expect(html).toContain('OpenCode')
    expect(html).toContain('Antigravity')
    expect(html).toContain('VS Code')
    expect(html).toContain('Zed')
    // Retired harnesses do not appear.
    expect(html).not.toContain('Claude Desktop')
    expect(html).not.toContain('Hermes')
    expect(html).not.toContain('Gemini CLI')
    expect(html).not.toContain('OpenClaw')
  })

  it('renders editorial state voices (silent success, mono uppercase action text)', () => {
    const html = renderApp()
    // Connect action link renders in mono uppercase (single word).
    expect(html).toMatch(/>\s*connect\s*/)
    // Connected state renders inline `connected` label + disconnect
    // link.
    expect(html).toMatch(/>\s*connected\s*/)
    expect(html).toMatch(/>\s*disconnect\s*/)
  })

  it('does NOT render the removed floating footer paragraph', () => {
    const html = renderApp()
    expect(html).not.toContain('Hermes and OpenClaw run inside BrowserOS')
  })

  it('does NOT render the removed Built-in variant', () => {
    const html = renderApp()
    expect(html).not.toContain('Built-in')
    expect(html).not.toContain('built-in')
  })

  it('does NOT render the removed marketing subtitle from the old HeroCard', () => {
    const html = renderApp()
    expect(html).not.toContain(
      'Add BrowserOS as an MCP server in your AI agent',
    )
    expect(html).not.toContain('One endpoint, every harness. Use the buttons')
  })
})
