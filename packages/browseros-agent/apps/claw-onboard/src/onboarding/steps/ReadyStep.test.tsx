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
  it('frames the step as the remaining connection work', () => {
    const html = render()

    expect(headingOf(html)).toContain('Last step:')
    expect(headingOf(html)).toContain('connect.')
    expect(html).toContain('Your agents can')
    expect(html).toContain('reach this browser yet')
  })

  it('spells out connect, restart, and paste as separate steps', () => {
    const html = render()

    expect(html).toContain('Open the MCP page and click')
    expect(html).toContain('Connect')
    expect(html).toContain('Restart Claude Code once')
    expect(html).toContain('Paste a task into Claude Code')
  })

  // The restart is the likeliest first-run failure: an agent that has not
  // reloaded its MCP config looks broken rather than unconnected.
  it('never drops the restart instruction', () => {
    expect(render()).toContain('picks up the connection')
  })

  it('names the paste destination rather than leaving it implied', () => {
    const html = render()

    expect(html).toContain('Claude Code')
    expect(html).not.toContain('Once connected, try one of these.')
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
