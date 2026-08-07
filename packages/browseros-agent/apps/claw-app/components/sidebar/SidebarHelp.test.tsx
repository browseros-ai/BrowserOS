import { afterEach, describe, expect, it, mock } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
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
        <SidebarHelp expanded />
      </TooltipProvider>,
    )
    expect(html).toContain('Help')
    expect(html).toContain('Docs')
    expect(html).toContain('Revisit Onboarding')
  })

  it('pins Docs and onboarding to their exact targets', () => {
    expect(helpItems.map((item) => [item.name, item.url])).toEqual([
      ['Docs', 'https://docs.browseros.com/browserclaw'],
      ['Revisit Onboarding', 'chrome://browseros-onboarding'],
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
