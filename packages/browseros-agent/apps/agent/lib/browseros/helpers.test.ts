import { beforeEach, describe, expect, it, mock } from 'bun:test'

const getPrefMock = mock(async () => ({ value: 9201 }))
const supportsMock = mock(async () => true)

mock.module('@/lib/env', () => ({
  env: {
    VITE_BROWSEROS_SERVER_PORT: 9110,
    VITE_ALPHA_FEATURES: false,
    PROD: false,
  },
}))

mock.module('./adapter', () => ({
  getBrowserOSAdapter: () => ({
    getPref: getPrefMock,
  }),
}))

mock.module('./capabilities', () => ({
  Capabilities: {
    supports: supportsMock,
  },
  Feature: {
    UNIFIED_PORT_SUPPORT: 'UNIFIED_PORT_SUPPORT',
    PROXY_SUPPORT: 'PROXY_SUPPORT',
  },
}))

describe('browseros helpers env precedence', () => {
  beforeEach(() => {
    getPrefMock.mockClear()
    supportsMock.mockClear()
  })

  it('prefers VITE_BROWSEROS_SERVER_PORT for unified agent URL', async () => {
    const { getAgentServerUrl } = await import('./helpers')
    await expect(getAgentServerUrl()).resolves.toBe('http://127.0.0.1:9110')
    expect(getPrefMock).not.toHaveBeenCalled()
  })

  it('prefers VITE_BROWSEROS_SERVER_PORT for MCP URL even when proxy is supported', async () => {
    const { getMcpServerUrl } = await import('./helpers')
    await expect(getMcpServerUrl()).resolves.toBe('http://127.0.0.1:9110/mcp')
    expect(getPrefMock).not.toHaveBeenCalled()
  })

  it('prefers VITE_BROWSEROS_SERVER_PORT for health URL even when proxy is supported', async () => {
    const { getHealthCheckUrl } = await import('./helpers')
    await expect(getHealthCheckUrl()).resolves.toBe('http://127.0.0.1:9110/health')
    expect(getPrefMock).not.toHaveBeenCalled()
  })
})
