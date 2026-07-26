import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

type TabsProps = ComponentProps<'div'> & {
  defaultValue?: string
  value?: string
}

mock.module('@/components/ui/button', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: MockButtonProps) =>
    createElement('button', { type: 'button', ...props }, children),
}))

mock.module('@/components/ui/tabs', () => ({
  Tabs: ({ children, defaultValue: _defaultValue, ...props }: TabsProps) =>
    createElement('div', props, children),
  TabsContent: ({ children, value: _value, ...props }: TabsProps) =>
    createElement('div', props, children),
  TabsList: ({ children, ...props }: ComponentProps<'div'>) =>
    createElement('div', props, children),
  TabsTrigger: ({ children, value: _value, ...props }: TabsProps) =>
    createElement('button', { type: 'button', ...props }, children),
}))

let QuickSetupSection: FC<{ serverUrl: string | null }>

beforeAll(async () => {
  QuickSetupSection = (await import('./QuickSetupSection')).QuickSetupSection
})

function render(serverUrl = 'http://127.0.0.1:9200/mcp'): string {
  return renderToStaticMarkup(createElement(QuickSetupSection, { serverUrl }))
}

describe('QuickSetupSection', () => {
  it('renders Claude Code setup with the served MCP URL', () => {
    const html = render()

    expect(html).toContain(
      'claude mcp add --transport http browseros http://127.0.0.1:9200/mcp --scope user',
    )
  })

  it('renders Claude Desktop setup as an mcp-remote command wrapper', () => {
    const html = render()

    expect(html).toContain('Claude Desktop')
    expect(html).toContain('Add this block to')
    expect(html).toContain('claude_desktop_config.json')
    expect(html).toContain('&quot;command&quot;: &quot;npx&quot;')
    expect(html).toContain('&quot;mcp-remote&quot;')
    expect(html).toContain('http://127.0.0.1:9200/mcp')

    const commandIndex = html.indexOf('&quot;command&quot;: &quot;npx&quot;')
    const desktopSnippet = html.slice(
      commandIndex,
      html.indexOf('Copy Claude Desktop setup', commandIndex),
    )
    expect(desktopSnippet).not.toContain(
      '&quot;url&quot;: &quot;http://127.0.0.1:9200/mcp&quot;',
    )
  })
})
