import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { SensitiveRouteContent } from './App'

describe('SensitiveRouteContent', () => {
  it('blocks an entire route body from PostHog replay capture', () => {
    const html = renderToStaticMarkup(
      <SensitiveRouteContent>
        <article>private task content</article>
      </SensitiveRouteContent>,
    )

    expect(html).toContain('class="ph-no-capture contents"')
    expect(html).toContain('private task content')
  })
})
