import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { TopSites } from './TopSites'
import type { TopSite } from './top-sites.hooks'

const sites: TopSite[] = [
  { url: 'https://news.kagi.com/', title: 'Kagi News' },
  { url: 'https://amazon.in/', title: 'Amazon' },
]

describe('TopSites', () => {
  it('renders a shortcut per site with a navigable href', () => {
    const html = renderToStaticMarkup(
      <TopSites isPending={false} sites={sites} />,
    )
    expect(html).toContain('href="https://news.kagi.com/"')
    expect(html).toContain('Kagi News')
    expect(html).toContain('href="https://amazon.in/"')
    expect(html).toContain('Amazon')
  })

  it('renders nothing on a fresh profile with no top sites', () => {
    const html = renderToStaticMarkup(<TopSites isPending={false} sites={[]} />)
    expect(html).toBe('')
  })

  it('renders a skeleton row while pending', () => {
    const html = renderToStaticMarkup(<TopSites isPending sites={[]} />)
    expect(html).toMatch(/animate-pulse/)
  })
})
