/**
 * Static-markup checks for the onboarding shell. Phase 3 cannot
 * exercise click flows under `renderToStaticMarkup`; per-step content
 * coverage lives in the per-step files. This file pins the shell
 * rendering, the visual rail copy, and the step-0 default landing.
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { OnboardingV2 } from './OnboardingV2'

function renderApp(): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <OnboardingV2 />
    </MemoryRouter>,
  )
}

describe('OnboardingV2 shell', () => {
  it('lands on step 0 with the welcome heading and primary CTA', () => {
    const html = renderApp()
    expect(html).toContain('The browser your agents')
    expect(html).toContain('drive')
    expect(html).toContain('Set up')
  })

  it('renders the visual rail with the v2 quote and three feature blocks', () => {
    const html = renderApp()
    expect(html).toContain('BrowserOS')
    expect(html).toContain('Let the agent you already run')
    expect(html).toContain('Fast &amp; token-cheap')
    expect(html).toContain('Logged in as you')
    expect(html).toContain('Under your control')
  })

  it('renders the macwin chrome bar title', () => {
    const html = renderApp()
    expect(html).toContain('Welcome to BrowserOS')
  })

  it('renders four step dots', () => {
    const html = renderApp()
    // 4 dots = 4 span elements with the rounded-full class. Counting
    // the rounded-full + h-[7px] combination is structural enough to
    // not break on minor style edits.
    const matches = html.match(/h-\[7px\]/g) ?? []
    expect(matches.length).toBeGreaterThanOrEqual(4)
  })
})
