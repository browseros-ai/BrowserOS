import { afterEach, describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { ThemeProvider } from '@/components/theme-provider'
import { TooltipProvider } from '@/components/ui/tooltip'
import { SidebarThemeToggle } from './SidebarThemeToggle'

// No localStorage in bun tests, so the provider always starts in
// system mode; interactive behavior (opening the menu, switching
// themes) is covered by the headless agent-browser pass.
function render(expanded: boolean): string {
  return renderToStaticMarkup(
    <TooltipProvider>
      <ThemeProvider>
        <SidebarThemeToggle expanded={expanded} />
      </ThemeProvider>
    </TooltipProvider>,
  )
}

const globals = globalThis as { localStorage?: unknown }

function stubStoredTheme(theme: string): void {
  globals.localStorage = {
    getItem: () => theme,
    setItem: () => {},
    removeItem: () => {},
  }
}

describe('SidebarThemeToggle', () => {
  afterEach(() => {
    delete globals.localStorage
  })

  it('labels the trigger with the current theme', () => {
    expect(render(false)).toContain('aria-label="Theme: System"')
  })

  it('renders the current theme icon', () => {
    expect(render(false)).toMatch(/<svg/)
  })

  it('shows the mode name when expanded', () => {
    expect(render(true)).toMatch(/opacity-100[^>]*>System</)
  })

  it('hides the mode name when collapsed', () => {
    expect(render(false)).toMatch(/opacity-0[^>]*>System</)
  })

  // The stored value stays 'dark' — only the label is the product word.
  it('labels the dark preference Gray, not Dark', () => {
    stubStoredTheme('dark')
    const markup = render(true)
    expect(markup).toContain('aria-label="Theme: Gray"')
    expect(markup).toMatch(/opacity-100[^>]*>Gray</)
    expect(markup).not.toContain('Dark')
  })
})
