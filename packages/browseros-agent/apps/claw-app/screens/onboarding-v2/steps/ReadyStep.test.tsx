import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { STARTER_PROMPTS } from '../onboarding-v2.helpers'
import { ReadyStep } from './ReadyStep'

function render(): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <ReadyStep onDone={() => undefined} />
    </MemoryRouter>,
  )
}

describe('ReadyStep', () => {
  it('renders the Open BrowserOS CTA', () => {
    expect(render()).toContain('Open BrowserOS')
  })

  it('renders the first two starter prompts from the fixture', () => {
    const html = render()
    expect(html).toContain(STARTER_PROMPTS[0])
    expect(html).toContain(STARTER_PROMPTS[1])
  })
})
