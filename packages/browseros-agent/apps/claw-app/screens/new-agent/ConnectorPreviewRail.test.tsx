import { describe, expect, it, mock } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import type { CreatedAgent } from '@/modules/api/agents.hooks'
import { newAgentDefaults } from './new-agent.schemas'

mock.module('react-hook-form', () => ({
  useFormContext: () => ({
    watch: () => ({
      ...newAgentDefaults,
      name: 'Demo connector',
    }),
  }),
}))

const { ConnectorPreviewRail } = await import('./ConnectorPreviewRail')

function render(createdAgent?: CreatedAgent): string {
  return renderToStaticMarkup(
    <ConnectorPreviewRail
      mode="create"
      createdAgent={createdAgent}
      isMutating={false}
      submitted={Boolean(createdAgent)}
      onDone={() => {}}
    />,
  )
}

describe('ConnectorPreviewRail', () => {
  it('does not expose a fallback MCP URL before pref-aware resolution completes', () => {
    const html = render()

    expect(html).toContain('MCP endpoint')
    expect(html).not.toContain('http://127.0.0.1:9200/mcp/demo-connector')
    expect(html).not.toContain('Copy MCP URL')
  })

  it('uses the server-created MCP URL after creation succeeds', () => {
    const html = render({
      id: 'agent-1',
      name: 'Demo connector',
      harness: 'Claude Code',
      slug: 'demo-connector',
      mcpUrl: 'http://127.0.0.1:9512/mcp/demo-connector',
      cliCommand: 'mcp add demo-connector',
      harnessInstall: {
        installed: true,
        message: 'Configured.',
      },
    })

    expect(html).toContain('http://127.0.0.1:9512/mcp/demo-connector')
    expect(html).toContain('Copy MCP URL')
  })
})
