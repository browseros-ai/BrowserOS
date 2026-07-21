import { describe, expect, test } from 'bun:test'
import { createAcpxProvider } from '../../src/index.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'

describe('AcpxProviderSettings.sessionOptions', () => {
  test('forwards systemPrompt string to runtime.ensureSession', async () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({
      agent: 'claude',
      runtime,
      sessionOptions: { systemPrompt: 'Be terse.' },
    })

    await provider.prepare()

    expect(runtime.ensureSessionCalls).toHaveLength(1)
    expect(runtime.ensureSessionCalls[0]?.sessionOptions).toEqual({
      systemPrompt: 'Be terse.',
    })
  })

  test('preserves { append } systemPrompt and other agent options', async () => {
    const runtime = new MockAcpRuntime()
    const sessionOptions = {
      systemPrompt: { append: 'Also review tests.' },
      model: 'fast',
      allowedTools: ['read', 'edit'],
      maxTurns: 5,
    }
    const provider = createAcpxProvider({
      agent: 'claude',
      runtime,
      sessionOptions,
    })

    await provider.prepare()

    expect(runtime.ensureSessionCalls[0]?.sessionOptions).toEqual(
      sessionOptions,
    )
  })

  test('omits sessionOptions when not configured', async () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    await provider.prepare()

    expect(runtime.ensureSessionCalls[0]?.sessionOptions).toBeUndefined()
  })
})
