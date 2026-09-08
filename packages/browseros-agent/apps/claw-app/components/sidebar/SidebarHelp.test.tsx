import { afterEach, describe, expect, it, mock } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { TooltipProvider } from '@/components/ui/tooltip'
import { helpItems, openHelpTarget, SidebarHelp } from './SidebarHelp'

const originalChrome = globalThis.chrome

afterEach(() => {
  Object.defineProperty(globalThis, 'chrome', {
    configurable: true,
    value: originalChrome,
  })
})

describe('SidebarHelp', () => {
  it('renders the help label and both entries when expanded', () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <MemoryRouter initialEntries={['/diagnostics']}>
          <SidebarHelp expanded />
        </MemoryRouter>
      </TooltipProvider>,
    )
    expect(html).toContain('Help')
    expect(html).toContain('Docs')
    expect(html).toContain('Revisit Onboarding')
    expect(html).toContain('Diagnostics')
    expect(html).toContain('aria-current="page"')
  })

  it('pins Docs and onboarding to their exact targets', () => {
    expect(helpItems.map((item) => [item.name, item.url])).toEqual([
      ['Docs', 'https://docs.browseros.com/browserclaw'],
      ['Revisit Onboarding', 'chrome://browseros-onboarding'],
      ['Diagnostics', '/diagnostics'],
    ])
  })

  it('opens a help target in a new tab via chrome.tabs.create', () => {
    const create = mock((_args: { url: string }) => Promise.resolve({}))
    Object.defineProperty(globalThis, 'chrome', {
      configurable: true,
      value: { tabs: { create } },
    })

    openHelpTarget('chrome://browseros-onboarding')

    expect(create).toHaveBeenCalledTimes(1)
    expect(create).toHaveBeenCalledWith({
      url: 'chrome://browseros-onboarding',
    })
  })
})
