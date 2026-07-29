import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { TopSites } from './TopSites'
import type { TopSite } from './top-sites.hooks'

const sites: TopSite[] = [
  { url: 'https://news.example.com/', title: 'Example News' },
  { url: 'https://shop.example.org/', title: 'Example Shop' },
]

describe('TopSites', () => {
  it('renders a navigable shortcut per site', () => {
    const html = renderToStaticMarkup(
      <TopSites sites={sites} isPending={false} />,
    )

    expect(html).toContain('href="https://news.example.com/"')
    expect(html).toContain('Example News')
    expect(html).toContain('href="https://shop.example.org/"')
    expect(html).toContain('Example Shop')
  })

  it('hides the row on a fresh profile with no history', () => {
    const html = renderToStaticMarkup(<TopSites sites={[]} isPending={false} />)

    expect(html).not.toContain('<a')
    expect(html).not.toContain('Top sites')
  })

  it('shows placeholder tiles while loading', () => {
    const html = renderToStaticMarkup(<TopSites sites={[]} isPending />)

    expect(html).toMatch(/animate-pulse/)
    expect(html).not.toContain('<a')
  })
})
