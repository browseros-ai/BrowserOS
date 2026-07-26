import { beforeAll, describe, expect, it, mock } from 'bun:test'

// Bun does not resolve WXT aliases, so register the full runtime export shape.
const browserosHelpersMock = {
  getAgentServerUrl: async () => 'http://127.0.0.1:9000',
  getMcpServerUrl: async () => 'http://127.0.0.1:9000/mcp',
  getProxyPort: async () => 9000,
  getHealthCheckUrl: async () => 'http://127.0.0.1:9000/system/health',
} satisfies typeof import('@/lib/browseros/helpers')
mock.module('@/lib/browseros/helpers', () => browserosHelpersMock)

let isAcpProbeEnabled: typeof import('./acp-probe.hooks').isAcpProbeEnabled
let resolveAcpAgentId: typeof import('./acp-probe.hooks').resolveAcpAgentId

beforeAll(async () => {
  ;({ isAcpProbeEnabled, resolveAcpAgentId } = await import(
    './acp-probe.hooks'
  ))
})

describe('resolveAcpAgentId', () => {
  it('returns the built-in claude id for claude-code', () => {
    expect(resolveAcpAgentId({ providerType: 'claude-code' })).toBe('claude')
  })

  it('returns the built-in codex id for codex', () => {
    expect(resolveAcpAgentId({ providerType: 'codex' })).toBe('codex')
  })

  it('returns undefined for acp-custom without an explicit acpAgentId', () => {
    expect(resolveAcpAgentId({ providerType: 'acp-custom' })).toBeUndefined()
  })

  it('honours an explicit acpAgentId override over the built-in default', () => {
    expect(
      resolveAcpAgentId({
        providerType: 'claude-code',
        acpAgentId: 'claude-experimental',
      }),
    ).toBe('claude-experimental')
  })

  it('returns undefined when providerType is missing', () => {
    expect(resolveAcpAgentId({ providerType: undefined })).toBeUndefined()
  })
})

describe('isAcpProbeEnabled', () => {
  const URL = 'http://127.0.0.1:9000'

  it('disables when providerType is missing', () => {
    expect(isAcpProbeEnabled({ providerType: undefined }, URL, 'claude')).toBe(
      false,
    )
  })

  it('disables when the agent server URL is missing', () => {
    expect(
      isAcpProbeEnabled({ providerType: 'claude-code' }, undefined, 'claude'),
    ).toBe(false)
  })

  it('disables when explicit enabled flag is false', () => {
    expect(
      isAcpProbeEnabled(
        { providerType: 'claude-code', enabled: false },
        URL,
        'claude',
      ),
    ).toBe(false)
  })

  it('enables for built-in claude-code with the resolved agent id', () => {
    expect(
      isAcpProbeEnabled({ providerType: 'claude-code' }, URL, 'claude'),
    ).toBe(true)
  })

  it('enables for built-in codex with the resolved agent id', () => {
    expect(isAcpProbeEnabled({ providerType: 'codex' }, URL, 'codex')).toBe(
      true,
    )
  })

  it('disables for acp-custom without a command', () => {
    expect(
      isAcpProbeEnabled(
        { providerType: 'acp-custom', acpAgentId: 'my-agent' },
        URL,
        'my-agent',
      ),
    ).toBe(false)
  })

  it('disables for acp-custom without an agentId', () => {
    expect(
      isAcpProbeEnabled(
        { providerType: 'acp-custom', command: 'my-bin acp' },
        URL,
        undefined,
      ),
    ).toBe(false)
  })

  it('enables for acp-custom with both command and agentId', () => {
    expect(
      isAcpProbeEnabled(
        {
          providerType: 'acp-custom',
          acpAgentId: 'my-agent',
          command: 'my-bin acp',
        },
        URL,
        'my-agent',
      ),
    ).toBe(true)
  })

  it('disables for built-in claude-code when agentId is somehow undefined', () => {
    expect(
      isAcpProbeEnabled({ providerType: 'claude-code' }, URL, undefined),
    ).toBe(false)
  })
})
