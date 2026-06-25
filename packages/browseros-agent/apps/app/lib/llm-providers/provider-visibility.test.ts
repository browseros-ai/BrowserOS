import { describe, expect, it } from 'bun:test'
import { Feature } from '../browseros/capabilities'
import {
  visibleProviderTemplates,
  visibleProviderTypeOptions,
} from './provider-visibility'
import { providerTemplates, providerTypeOptions } from './providerTemplates'

function supportsHermes(enabled: boolean) {
  return (feature: Feature) =>
    feature === Feature.HERMES_AGENT_SUPPORT ? enabled : true
}

describe('provider visibility', () => {
  it('hides Remote Hermes when Hermes support is off', () => {
    const templates = visibleProviderTemplates(
      providerTemplates,
      supportsHermes(false),
    )
    const options = visibleProviderTypeOptions(
      providerTypeOptions,
      supportsHermes(false),
    )

    expect(templates.map((template) => template.id)).not.toContain(
      'remote-hermes',
    )
    expect(options.map((option) => option.value)).not.toContain('remote-hermes')
    expect(templates.map((template) => template.id)).toContain('openai')
  })

  it('keeps Remote Hermes visible when Hermes support is on', () => {
    const templates = visibleProviderTemplates(
      providerTemplates,
      supportsHermes(true),
    )
    const options = visibleProviderTypeOptions(
      providerTypeOptions,
      supportsHermes(true),
    )

    expect(templates.map((template) => template.id)).toContain('remote-hermes')
    expect(options.map((option) => option.value)).toContain('remote-hermes')
  })
})
