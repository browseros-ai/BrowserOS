/**
 * Pins the editorial MCP page shape: compressed hero + single
 * endpoint URL strip (CLI snippet removed), inline Connected-agents
 * header with an `N of M connected` mono chip, hairline row list
 * filtered to exclude Hermes / OpenClaw / Gemini CLI.
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
          // Claude Desktop is a retired harness (stdio-only host
          // config, `npx mcp-remote` bridge requires Node) and gets
          // filtered out at the render layer; the row list should
          // not include it.
          {
            harness: 'Claude Desktop',
            installed: false,
            agentId: 'claude-desktop',
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
          // Hermes is BrowserOS-internal and gets filtered out at the
          // render layer; the row list should not include it.
          {
            harness: 'Hermes',
            installed: true,
            agentId: null,
            message: 'Runs inside BrowserOS.',
          },
          // Gemini CLI is also filtered out at the render layer.
          {
            harness: 'Gemini CLI',
            installed: false,
            agentId: 'gemini-cli',
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
  it('renders the editorial hero: H1 + italic accent line + single slugless endpoint URL', () => {
    const html = renderApp()
    expect(html).toContain('MCP')
    // Italic accent line signature (matches cockpit's `working on`).
    expect(html).toContain('every')
    expect(html).toContain('harness.')
    // Endpoint URL renders and is not slugged.
    expect(html).toContain('/mcp')
    expect(html).not.toContain('/mcp/claude-code')
    expect(html).not.toContain('/cockpit')
    // Copy affordance renders.
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

  it('renders the Connected-agents header with the filtered-count chip', () => {
    const html = renderApp()
    expect(html).toContain('Connected agents')
    // Visible list = Claude Code, Cursor, Codex (Hermes + Gemini CLI
    // filtered). Connected among visible = 1 (Cursor).
    expect(html).toContain('1 of 3 connected')
  })

  it('renders one row per visible harness and hides the filtered ones', () => {
    const html = renderApp()
    expect(html).toContain('Claude Code')
    expect(html).toContain('Cursor')
    expect(html).toContain('Codex')
    // Filtered out at the render layer.
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

  it('does NOT render the legacy McpRow / RegenerateUrlDialog / "Add agent" CTA', () => {
    const html = renderApp()
    expect(html).not.toContain('Add agent')
    expect(html).not.toContain('Regenerate URL')
    expect(html).not.toContain('No endpoints yet')
  })

  it('does NOT render the removed marketing subtitle from the old HeroCard', () => {
    const html = renderApp()
    expect(html).not.toContain(
      'Add BrowserOS as an MCP server in your AI agent',
    )
    expect(html).not.toContain('One endpoint, every harness. Use the buttons')
  })
})
