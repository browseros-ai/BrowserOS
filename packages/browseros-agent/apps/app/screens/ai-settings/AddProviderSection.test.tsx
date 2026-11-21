import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

mock.module('@/components/agents/AdapterIcon', () => ({
  AdapterIcon: () => createElement('span', { 'data-icon': 'adapter' }),
  adapterLabel: (type: string) => (type === 'codex' ? 'Codex' : 'Claude Code'),
}))
mock.module('@/lib/llm-providers/providerIcons', () => ({
  ProviderIcon: () => createElement('span', { 'data-icon': 'provider' }),
}))
mock.module('@/modules/browseros/capabilities.hooks', () => ({
  useCapabilities: () => ({ supports: () => true }),
}))

let AddProviderSection: FC<
  import('./AddProviderSection').AddProviderSectionProps
>

beforeAll(async () => {
  AddProviderSection = (await import('./AddProviderSection')).AddProviderSection
})

function render() {
  return renderToStaticMarkup(
    createElement(AddProviderSection, {
      onCreateAgent: () => {},
      onUseTemplate: () => {},
    }),
  )
}

describe('AddProviderSection', () => {
  it('renders every category heading', () => {
    const html = render()
    for (const heading of [
      'Coding agents',
      'Sign in with a subscription',
      'Bring your own API key',
      'Runs on this machine',
    ]) {
      expect(html).toContain(heading)
    }
  })

  it('puts the coding agents ahead of the provider templates', () => {
    const html = render()
    expect(html.indexOf('Coding agents')).toBeLessThan(
      html.indexOf('Bring your own API key'),
    )
  })

  it('offers Fireworks and omits retired targets', () => {
    const html = render()
    expect(html).toContain('Add Fireworks')
    for (const name of ['Custom ACP agent', 'GitHub Copilot', 'Qwen Code']) {
      expect(html).not.toContain(name)
    }
  })

  it('groups the on-device runtimes under the local heading', () => {
    const html = render()
    const localStart = html.indexOf('Runs on this machine')
    expect(html.indexOf('Ollama')).toBeGreaterThan(localStart)
    expect(html.indexOf('LMStudio')).toBeGreaterThan(localStart)
  })

  // One verb for every entry: the old grid mixed USE and ADD badges.
  it('labels every entry with the same action', () => {
    const html = render()
    expect(html).not.toContain('>USE<')
    expect(html).not.toContain('>ADD<')
    expect((html.match(/>Add</g) ?? []).length).toBeGreaterThan(10)
  })

  it('names the target in each button accessible name', () => {
    const html = render()
    expect(html).toContain('aria-label="Add OpenAI"')
    expect(html).toContain('aria-label="Add Ollama"')
  })

  it('gives the search field a real label rather than a placeholder', () => {
    const html = render()
    expect(html).toContain('Search providers and agents')
    expect(html).toMatch(/<label[^>]*for="[^"]+"/)
  })

  it('keeps a brand icon on every tile', () => {
    const html = render()
    expect(html).toContain('data-icon="provider"')
    expect(html).toContain('data-icon="adapter"')
  })
})
