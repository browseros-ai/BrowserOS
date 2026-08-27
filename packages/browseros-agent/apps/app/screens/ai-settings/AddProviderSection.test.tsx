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
mock.module('@/components/agents/agent-brand-marks', () => ({
  BRAND_MARKS: {
    opencode: () => createElement('span', { 'data-mark': 'opencode' }),
    hermes: () => createElement('span', { 'data-mark': 'hermes' }),
    openclaw: () => createElement('span', { 'data-mark': 'openclaw' }),
    pi: () => createElement('span', { 'data-mark': 'pi' }),
    antigravity: () => createElement('span', { 'data-mark': 'antigravity' }),
  },
  agentBrandKey: () => undefined,
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
      onCreateCustomAgent: () => {},
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

  // The old grid injected agent tiles inside the provider loop, which put the
  // custom-agent tile in the middle of the providers.
  it('keeps the custom agent tile inside the coding agents group', () => {
    const html = render()
    const agentsStart = html.indexOf('Coding agents')
    const nextGroup = html.indexOf('Sign in with a subscription')
    const custom = html.indexOf('Custom ACP agent')
    expect(custom).toBeGreaterThan(agentsStart)
    expect(custom).toBeLessThan(nextGroup)
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
    expect(html).toContain('aria-label="Add Custom ACP agent"')
  })

  it('gives the search field a real label rather than a placeholder', () => {
    const html = render()
    expect(html).toContain('Search providers and agents')
    expect(html).toMatch(/<label[^>]*for="[^"]+"/)
  })

  // "Custom ACP agent" says what it is, not what it gets you. The marks of the
  // agents the picker offers are what make the tile worth clicking.
  it('shows the popular agent marks on the custom agent tile', () => {
    const html = render()
    for (const id of ['opencode', 'hermes', 'openclaw', 'pi', 'antigravity']) {
      expect(html).toContain(`data-mark="${id}"`)
    }
  })

  it('names those agents in the custom tile copy', () => {
    const html = render()
    expect(html).toContain('opencode')
    expect(html).toContain('OpenClaw')
    expect(html).toContain('any other ACP compatible agent')
  })

  it('keeps a brand icon on every tile', () => {
    const html = render()
    expect(html).toContain('data-icon="provider"')
    expect(html).toContain('data-icon="adapter"')
  })
})
