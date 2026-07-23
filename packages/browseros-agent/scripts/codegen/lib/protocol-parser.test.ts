import { describe, expect, it } from 'bun:test'
import { parseProtocol } from './protocol-parser'

describe('parseProtocol', () => {
  it('parses a combined protocol document', () => {
    const protocol = parseProtocol(
      JSON.stringify({
        version: { major: '1', minor: '3' },
        domains: [{ domain: 'Browser' }, { domain: 'Runtime' }],
      }),
    )

    expect(protocol).toEqual({
      version: { major: '1', minor: '3' },
      domains: [{ domain: 'Browser' }, { domain: 'Runtime' }],
    })
  })

  it('rejects malformed JSON with a clear error', () => {
    expect(() => parseProtocol('{')).toThrow('Invalid protocol JSON:')
  })

  it('rejects documents without the protocol envelope', () => {
    expect(() => parseProtocol('{}')).toThrow(
      'Invalid protocol JSON: missing version or domains',
    )
  })

  it('rejects duplicate domain names', () => {
    expect(() =>
      parseProtocol(
        JSON.stringify({
          version: { major: '1', minor: '3' },
          domains: [{ domain: 'Browser' }, { domain: 'Browser' }],
        }),
      ),
    ).toThrow('Invalid protocol JSON: duplicate domain Browser')
  })
})
