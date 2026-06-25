import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { ConnectPhase } from '../onboarding-v2.types'
import { ConnectStep } from './ConnectStep'

function render(phase: ConnectPhase): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <ConnectStep
        phase={phase}
        onAddToClaude={() => undefined}
        onContinue={() => undefined}
      />
    </MemoryRouter>,
  )
}

describe('ConnectStep', () => {
  it('renders the Add-to-Claude CTA and the canonical CLI snippet in idle phase', () => {
    const html = render('idle')
    expect(html).toContain('Add to Claude')
    expect(html).toContain('or use the CLI')
    expect(html).toContain('claude mcp add browseros')
    expect(html).toContain('--transport http')
  })

  it('shows the Connecting state and disables the button while connecting', () => {
    const html = render('connecting')
    expect(html).toContain('Connecting')
    expect(html).toContain('disabled')
  })

  it('renders the success card and Continue CTA when connected', () => {
    const html = render('connected')
    expect(html).toContain('Connected to Claude')
    expect(html).toContain('68 browser tools available')
    expect(html).toContain('You')
    expect(html).toContain('set')
  })
})
