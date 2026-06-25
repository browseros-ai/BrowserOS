import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { EmptyState } from './EmptyState'

describe('EmptyState', () => {
  it('renders the title and hint', () => {
    const html = renderToStaticMarkup(
      <EmptyState title="No agents connected" hint="Open the MCP page." />,
    )
    expect(html).toContain('No agents connected')
    expect(html).toContain('Open the MCP page.')
  })

  it('uses the default icon when none is supplied', () => {
    const html = renderToStaticMarkup(<EmptyState title="t" hint="h" />)
    // The default Activity icon from lucide renders an <svg>. We do
    // not assert which icon; just that an svg is present.
    expect(html).toMatch(/<svg/)
  })
})
