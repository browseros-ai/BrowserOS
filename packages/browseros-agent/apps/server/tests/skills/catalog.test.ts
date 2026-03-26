import { describe, expect, it } from 'bun:test'
import { buildSkillsCatalog } from '../../src/skills/catalog'
import type { SkillMeta } from '../../src/skills/types'

const sampleSkill: SkillMeta = {
  id: 'write-docs',
  name: 'Write docs',
  description: 'Mintlify-style documentation.',
  location: '/skills/write-docs/SKILL.md',
  enabled: true,
  builtIn: true,
}

describe('buildSkillsCatalog', () => {
  it('instructs filesystem_read in regular mode', () => {
    const cat = buildSkillsCatalog([sampleSkill])
    expect(cat).toContain('filesystem_read')
    expect(cat).toContain('<location>')
    expect(cat).toContain('/skills/write-docs/SKILL.md')
  })

  it('omits filesystem_read and locations in chat mode', () => {
    const cat = buildSkillsCatalog([sampleSkill], { chatMode: true })
    expect(cat).not.toContain('filesystem_read')
    expect(cat).not.toContain('<location>')
    expect(cat).toContain('read-only chat mode')
    expect(cat).toContain('Write docs')
    expect(cat).toContain('Mintlify-style documentation.')
  })
})
