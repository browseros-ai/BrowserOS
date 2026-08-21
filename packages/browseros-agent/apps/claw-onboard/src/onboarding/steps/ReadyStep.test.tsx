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

function headingOf(html: string): string {
  const start = html.indexOf('<h1')
  const end = html.indexOf('</h1>')
  if (start === -1 || end === -1) return ''
  return html.slice(start, end + '</h1>'.length)
}

describe('ReadyStep', () => {
  it('frames the step as finished rather than as remaining work', () => {
    const html = render()

    expect(headingOf(html)).toContain('All')
    expect(headingOf(html)).toContain('set!')
    expect(headingOf(html)).not.toContain('Last step:')
    expect(html).not.toContain('Three things')
  })

  // The browser connects itself to the agents already on this machine, so the
  // screen states that; a checklist would ask for work that is already done.
  it('states the connection already happened instead of listing steps', () => {
    const html = render()

    expect(html).toContain('already installed on your machine')
    expect(html).not.toContain('<ol')
    expect(html).not.toContain('Open the MCP page and click')
  })

  // The restart is the likeliest first-run failure: an agent that has not
  // reloaded its MCP config looks broken rather than unconnected. It stays, but
  // below the prompts, so a finished setup does not read as unfinished.
  it('keeps the restart as a footnote under the prompts, never dropping it', () => {
    const html = render()

    expect(html).toContain('picks up the connection')
    expect(html.indexOf('Restart it once')).toBeGreaterThan(
      html.indexOf(STARTER_PROMPTS[0]),
    )
  })

  // Every installed agent is connected, so the paste target is whichever one
  // the reader uses — naming Claude Code would now be wrong for most of them.
  it('sends the prompt to whichever agent the reader uses, not a named tool', () => {
    const html = render()

    expect(html).toContain('into your agent')
    expect(html).not.toContain('Claude Code')
  })

  it('renders the MCP page CTA without claiming to connect anything', () => {
    const html = render()

    expect(html).toContain('Open the MCP page')
    expect(html).not.toContain('Connect your AI')
  })

  it('no longer claims an import happened', () => {
    const html = render()

    expect(html).not.toContain('Logins')
    expect(html).not.toContain('imported')
  })

  it('renders the starter prompts', () => {
    const html = render()

    expect(html).toContain(STARTER_PROMPTS[0])
    expect(html).toContain(STARTER_PROMPTS[1])
  })
})
