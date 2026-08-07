import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test'
import type {
  AcpPermissionDecision,
  AcpPermissionRequest,
  AcpRuntime,
  AcpRuntimeOptions,
} from 'acpx/runtime'

const createAcpRuntimeMock = mock(
  (_options: AcpRuntimeOptions): AcpRuntime =>
    ({
      ensureSession: async () => ({}),
      startTurn: () => ({
        requestId: 'r',
        events: { [Symbol.asyncIterator]: async function* () {} },
        result: Promise.resolve({ status: 'completed' }),
        cancel: async () => {},
        closeStream: async () => {},
      }),
      cancel: async () => {},
      close: async () => {},
    }) as unknown as AcpRuntime,
)
const createAgentRegistryMock = mock(() => ({
  resolve: () => ['codex'],
  list: () => ['codex'],
}))

mock.module('acpx/runtime', () => ({
  createAcpRuntime: createAcpRuntimeMock,
  createAgentRegistry: createAgentRegistryMock,
  createFileSessionStore: () => ({}),
}))

// Imported AFTER `mock.module` so the provider sees our stubs.
const { createAcpxProvider } = await import('../../src/provider.ts')

beforeEach(() => {
  createAcpRuntimeMock.mockClear()
  createAgentRegistryMock.mockClear()
})

afterEach(() => {
  createAcpRuntimeMock.mockClear()
  createAgentRegistryMock.mockClear()
})

describe('AcpxProvider — onPermissionRequest', () => {
  test('forwards structured agent argv to the ACPX registry', () => {
    const argv = ['env', 'INITIAL_AGENT_MODE=agent-full-access', 'codex-acp']
    const provider = createAcpxProvider({
      agent: 'codex',
      agentRegistryOverrides: { codex: argv },
    })

    void provider.runtime

    expect(createAgentRegistryMock).toHaveBeenCalledWith({
      overrides: { codex: argv },
    })
  })

  test('forwards the callback into AcpRuntimeOptions', () => {
    const cb = async (
      _req: AcpPermissionRequest,
    ): Promise<AcpPermissionDecision | undefined> => undefined
    const provider = createAcpxProvider({
      agent: 'codex',
      onPermissionRequest: cb,
    })
    void provider.runtime // force lazy build

    expect(createAcpRuntimeMock).toHaveBeenCalledTimes(1)
    const opts = createAcpRuntimeMock.mock.calls[0]?.[0]
    expect(opts?.onPermissionRequest).toBe(cb)
  })

  test('omits the callback when not configured', () => {
    const provider = createAcpxProvider({ agent: 'codex' })
    void provider.runtime

    const opts = createAcpRuntimeMock.mock.calls.at(-1)?.[0]
    expect(opts?.onPermissionRequest).toBeUndefined()
  })

  test('skips runtime construction when a pre-built runtime is provided', () => {
    const fakeRuntime = {
      ensureSession: async () => ({}),
    } as unknown as AcpRuntime
    const cb = async (
      _req: AcpPermissionRequest,
    ): Promise<AcpPermissionDecision | undefined> => undefined

    const provider = createAcpxProvider({
      agent: 'codex',
      runtime: fakeRuntime,
      onPermissionRequest: cb,
    })

    expect(provider.runtime).toBe(fakeRuntime)
    expect(createAcpRuntimeMock).not.toHaveBeenCalled()
  })
})
