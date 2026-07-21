import { describe, expect, test } from 'bun:test'
import { AcpxProvider, createAcpxProvider, VERSION } from '../../src/index'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime'

describe('package surface', () => {
  test('exports VERSION sentinel', () => {
    expect(VERSION).toBe('0.0.0')
  })

  test('createAcpxProvider returns an AcpxProvider instance', () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    expect(provider).toBeInstanceOf(AcpxProvider)
  })
})
