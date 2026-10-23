import { describe, expect, it } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter, Route, Routes } from 'react-router'
import { ThemeProvider } from '@/components/theme-provider'
import { TooltipProvider } from '@/components/ui/tooltip'
import { AppLayout } from './AppLayout'
import { Page } from './Page'

function renderAt(path: string) {
  const html = renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <TooltipProvider>
        <ThemeProvider>
          <MemoryRouter initialEntries={[path]}>
            <Routes>
              <Route element={<AppLayout />}>
                <Route
                  path="/"
                  element={
                    <Page>
                      <h1>Home page</h1>
                    </Page>
                  }
                />
                <Route
                  path="/audit"
                  element={
                    <Page>
                      <h1>History page</h1>
                    </Page>
                  }
                />
                <Route
                  path="/audit/:sessionId/*"
                  element={
                    <Page>
                      <h1>Task page</h1>
                    </Page>
                  }
                />
              </Route>
            </Routes>
          </MemoryRouter>
        </ThemeProvider>
      </TooltipProvider>
    </QueryClientProvider>,
  )
  return parseHTML(`<!doctype html><html><body>${html}</body></html>`).document
}

function linkTexts(nav: Element | null | undefined): string[] {
  return [...(nav?.querySelectorAll('a') ?? [])].map(
    (link) => link.textContent?.trim() ?? '',
  )
}

describe('AppLayout', () => {
  it('renders the routed page inside one main region with one page frame', () => {
    const document = renderAt('/audit')
    const main = document.querySelectorAll('main')
    expect(main).toHaveLength(1)
    expect(main[0]?.querySelectorAll('[data-page]')).toHaveLength(1)
    expect(main[0]?.textContent).toContain('History page')
  })

  it('lists the destinations with words, in the agreed order', () => {
    const document = renderAt('/')
    const navs = [...document.querySelectorAll('nav')]
    const main = navs.find((nav) => nav.getAttribute('aria-label') === 'Main')
    const settings = navs.find(
      (nav) => nav.getAttribute('aria-label') === 'Settings',
    )
    expect(linkTexts(main)).toEqual([
      'Home',
      'Connected agents',
      'History',
      'Saved tasks',
    ])
    expect(linkTexts(settings)).toEqual(['Settings'])
  })

  it('marks the section of a nested page as current', () => {
    const document = renderAt('/audit/session-1')
    const current = document.querySelector('a[aria-current="page"]')
    expect(current?.textContent?.trim()).toBe('History')
  })

  it('treats a task and its replay deep link as the same page', () => {
    const task = renderAt('/audit/session-1')
    const replay = renderAt('/audit/session-1/replay')
    const key = (document: Document) =>
      document.querySelector('[data-page-key]')?.getAttribute('data-page-key')
    expect(key(task)).toBe('/audit/session-1')
    expect(key(replay)).toBe('/audit/session-1')
  })
})
