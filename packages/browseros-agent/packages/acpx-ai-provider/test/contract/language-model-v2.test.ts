import { describe, expect, test } from 'bun:test'
import type { LanguageModelV2 } from '@ai-sdk/provider'
import { type AcpxLanguageModel, createAcpxProvider } from '../../src/index.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'

// Compile-time conformance check. If AI SDK ever ships a breaking change
// to LanguageModelV2 that AcpxLanguageModel no longer satisfies, this
// file fails to typecheck — surfacing the drift before users find it.
type _AssertConforms = AcpxLanguageModel extends LanguageModelV2 ? true : never
const _conforms: _AssertConforms = true
void _conforms

function newModel() {
  const runtime = new MockAcpRuntime()
  return createAcpxProvider({ agent: 'claude', runtime }).languageModel()
}

describe('LanguageModelV2 contract', () => {
  test('specificationVersion is "v2"', () => {
    expect(newModel().specificationVersion).toBe('v2')
  })

  test('provider is "acpx"', () => {
    expect(newModel().provider).toBe('acpx')
  })

  test('modelId reflects the configured agent', () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({ agent: 'codex', runtime })
    expect(provider.languageModel().modelId).toBe('codex')
  })

  test('per-instance agent override changes the modelId', () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    expect(provider.languageModel(undefined, { agent: 'codex' }).modelId).toBe(
      'codex',
    )
  })

  test('supportedUrls is an empty record', () => {
    expect(newModel().supportedUrls).toEqual({})
  })

  test('doGenerate is a callable function', () => {
    expect(typeof newModel().doGenerate).toBe('function')
  })

  test('doStream is a callable function', () => {
    expect(typeof newModel().doStream).toBe('function')
  })
})
