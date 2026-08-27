import { describe, expect, it } from 'bun:test'
import {
  buildCustomConfig,
  formatEnvLines,
  parseCsv,
  parseEnvLines,
} from './custom-agent-form.helpers'

describe('parseEnvLines', () => {
  it('parses KEY=value lines and skips blanks and comments', () => {
    expect(parseEnvLines('A=1\n\n# note\nB = two = three ')).toEqual({
      A: '1',
      B: 'two = three',
    })
  })

  it('ignores lines without a key', () => {
    expect(parseEnvLines('=nope\nGOOD=1')).toEqual({ GOOD: '1' })
  })
})

describe('formatEnvLines', () => {
  it('round-trips with parseEnvLines', () => {
    const env = { A: '1', B: 'two' }
    expect(parseEnvLines(formatEnvLines(env))).toEqual(env)
  })

  it('returns empty string for undefined', () => {
    expect(formatEnvLines(undefined)).toBe('')
  })
})

describe('parseCsv', () => {
  it('splits, trims, and drops empties', () => {
    expect(parseCsv(' a, b ,,c ')).toEqual(['a', 'b', 'c'])
  })
})

describe('buildCustomConfig', () => {
  it('includes only the fields that are set', () => {
    expect(
      buildCustomConfig({
        command: '  npx -y @scope/agent  ',
        envText: '',
        fullAccessModesText: '',
        reasoningEffortKey: '',
        systemPromptAppend: '',
        icon: '',
      }),
    ).toEqual({ command: 'npx -y @scope/agent' })
  })

  it('carries env, modes, reasoning key, prompt, and icon', () => {
    expect(
      buildCustomConfig({
        command: 'my-agent',
        envText: 'KEY=v',
        fullAccessModesText: 'bypass, full',
        reasoningEffortKey: ' effort ',
        systemPromptAppend: ' extra ',
        icon: ' 🤖 ',
      }),
    ).toEqual({
      command: 'my-agent',
      env: { KEY: 'v' },
      fullAccessModes: ['bypass', 'full'],
      reasoningEffortKey: 'effort',
      systemPromptAppend: 'extra',
      icon: '🤖',
    })
  })
})
