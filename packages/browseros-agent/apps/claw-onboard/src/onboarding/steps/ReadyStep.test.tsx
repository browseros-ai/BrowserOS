import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { STARTER_PROMPTS } from '../onboarding-v2.helpers'
import { ReadyStep } from './ReadyStep'

function render(imported = true): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <ReadyStep imported={imported} onDone={() => undefined} />
    </MemoryRouter>,
  )
}

// The "Connect your AI" CTA would satisfy a whole-document match for either
// heading variant, so heading assertions have to be scoped to the <h1>.
function headingOf(html: string): string {
  const start = html.indexOf('<h1')
  const end = html.indexOf('</h1>')
  if (start === -1 || end === -1) return ''
  return html.slice(start, end + '</h1>'.length)
}

describe('ReadyStep', () => {
  it('confirms imported logins before pointing to MCP setup', () => {
    const html = render()

    expect(headingOf(html)).toContain('Logins')
    expect(headingOf(html)).toContain('imported')
    expect(html).toContain('One step left.')
    expect(html).toContain('Open the MCP page in BrowserClaw')
    expect(html).toContain('Claude Code, Cursor, Codex')
    expect(html).toContain('You watch, approve, and audit.')
  })

  it('does not claim an import happened when the step was skipped', () => {
    const heading = headingOf(render(false))

    expect(heading).toContain('Connect your')
    expect(heading).toContain('AI.')
    expect(heading).not.toContain('Logins')
    expect(heading).not.toContain('imported')
  })

  it('renders the MCP setup CTA', () => {
    expect(render()).toContain('Connect your AI')
    expect(render(false)).toContain('Connect your AI')
  })

  it('keeps the MCP copy identical whether or not the import ran', () => {
    const html = render(false)

    expect(html).toContain('One step left.')
    expect(html).toContain('Open the MCP page in BrowserClaw')
    expect(html).toContain('You watch, approve, and audit.')
  })

  it('frames starter prompts as post-connection examples', () => {
    const html = render()
    expect(html).toContain('Once connected, try one of these.')
    expect(html).toContain(STARTER_PROMPTS[0])
    expect(html).toContain(STARTER_PROMPTS[1])
  })
})
