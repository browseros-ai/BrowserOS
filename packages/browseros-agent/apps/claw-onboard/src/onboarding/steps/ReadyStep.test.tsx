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
  it('confirms imported logins before pointing to MCP setup', () => {
    const html = render()

    expect(html).toContain('Logins')
    expect(html).toContain('imported')
    expect(html).toContain('One step left.')
    expect(html).toContain('Open the MCP page in BrowserClaw')
    expect(html).toContain('Claude Code, Cursor, Codex')
    expect(html).toContain('You watch, approve, and audit.')
  })

  it('renders the MCP setup CTA', () => {
    expect(render()).toContain('Connect your AI')
  })

  it('frames starter prompts as post-connection examples', () => {
    const html = render()
    expect(html).toContain('Once connected, try one of these.')
    expect(html).toContain(STARTER_PROMPTS[0])
    expect(html).toContain(STARTER_PROMPTS[1])
  })
})
