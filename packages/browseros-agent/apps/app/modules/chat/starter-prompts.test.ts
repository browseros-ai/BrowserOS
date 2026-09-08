import { describe, expect, it } from 'bun:test'
import {
  getDefaultStarterPrompts,
  resolveStarterPrompts,
  starterPromptsSchema,
} from './starter-prompts'

describe('starter prompt preferences', () => {
  it('falls back to usable defaults for missing or malformed saved data', () => {
    for (const value of [
      null,
      {},
      [],
      [{ display: 'Only one', prompt: 'Go' }],
    ]) {
      expect(resolveStarterPrompts('agent', value)).toEqual(
        getDefaultStarterPrompts('agent'),
      )
    }
  })

  it('rejects incomplete or oversized shortcuts before saving', () => {
    for (const change of [
      { display: '   ' },
      { prompt: '\n ' },
      { display: 'x'.repeat(81) },
      { prompt: 'x'.repeat(4001) },
    ]) {
      const prompts = getDefaultStarterPrompts('chat')
      prompts[1] = { ...prompts[1], ...change }
      expect(starterPromptsSchema.safeParse(prompts).success).toBe(false)
    }
  })

  it('preserves multiline instructions and allows identical button labels', () => {
    const prompts = getDefaultStarterPrompts('agent').map((prompt) => ({
      ...prompt,
      display: ' My shortcut ',
      prompt: ' Read this page.\n\nReturn three bullets. ',
    }))
    expect(starterPromptsSchema.parse(prompts)).toEqual(
      prompts.map(() => ({
        display: 'My shortcut',
        prompt: 'Read this page.\n\nReturn three bullets.',
      })),
    )
  })

  it('returns independent defaults so an editing draft cannot change them', () => {
    const prompts = getDefaultStarterPrompts('agent')
    prompts[0].display = 'My draft'
    expect(getDefaultStarterPrompts('agent')[0].display).toBe(
      'Summarize this page',
    )
    expect(getDefaultStarterPrompts('chat')[1].display).toBe(
      'What topics does this page talk about?',
    )
  })
})
