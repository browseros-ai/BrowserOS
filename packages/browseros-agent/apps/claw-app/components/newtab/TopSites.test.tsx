import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { TopSites } from './TopSites'
import type { TopSite } from './top-sites.hooks'

const sites: TopSite[] = [
  { url: 'https://github.com/', title: 'GitHub' },
  { url: 'https://news.ycombinator.com/', title: '' },
]

describe('TopSites', () => {
  it('renders one navigable tile per site, targeting the current tab', () => {
    const html = renderToStaticMarkup(
      <TopSites topSites={sites} pending={false} />,
    )
    expect(html).toContain('href="https://github.com/"')
    expect(html).toContain('data-top-site="https://github.com/"')
    expect(html).toContain('GitHub')
    // A blank title falls back to the hostname rather than an empty label.
    expect(html).toContain('news.ycombinator.com')
    expect(html).not.toContain('target="_blank"')
  })

  it('shows a skeleton row while loading and no real tiles', () => {
    const html = renderToStaticMarkup(<TopSites topSites={[]} pending={true} />)
    expect(html).not.toContain('data-top-site')
    expect(html).toContain('animate-pulse')
  })

  it('renders nothing on a fresh profile with no history', () => {
    const html = renderToStaticMarkup(
      <TopSites topSites={[]} pending={false} />,
    )
    expect(html).toBe('')
  })
})
