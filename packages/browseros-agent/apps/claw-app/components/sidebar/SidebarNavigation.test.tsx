import { describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { TooltipProvider } from '@/components/ui/tooltip'
import { SidebarNavigation } from './SidebarNavigation'

describe('SidebarNavigation', () => {
  it('tests that the sidebar links to Settings and marks it current there', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <MemoryRouter initialEntries={['/settings']}>
          <SidebarNavigation expanded />
        </MemoryRouter>
      </TooltipProvider>,
    )
    const page = parseHTML(`<!doctype html><html><body>${html}</body></html>`)
    const link = page.document.querySelector('a[href="/settings"]')

    expect(link?.textContent).toBe('Settings')
    expect(link?.getAttribute('aria-current')).toBe('page')
  })
})
