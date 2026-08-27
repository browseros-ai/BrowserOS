/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { dirname } from 'node:path'
import { HOST_ACP_ADAPTER_CONFIG } from '../../../../src/lib/agents/host-acp/config'
import { resolveAcpSpawnCommand } from '../../../../src/lib/agents/host-acp/launcher'

const FAKE_BUN_PATH = '/Volumes/BrowserOS/bin/third_party/bun'
const WINDOWS_BUN_PATH =
  'C:\\Users\\shadowfax\\AppData\\Local\\BrowserOS\\Application\\148.0.7947.97\\BrowserOSServer\\default\\resources\\bin\\third_party\\bun.exe'

const stubBunPresent: typeof import('../../../../src/lib/agents/host-acp/bundled-bun').resolveBundledBun =
  () => FAKE_BUN_PATH

const stubBunMissing: typeof import('../../../../src/lib/agents/host-acp/bundled-bun').resolveBundledBun =
  () => null

function decodeEnvironmentPayload(value: string | undefined): {
  argv: string[]
  env: Record<string, string>
} {
  return JSON.parse(Buffer.from(value ?? '', 'base64url').toString('utf8'))
}

describe('resolveAcpSpawnCommand', () => {
  it('returns the bundled-bun launcher for claude when the binary exists', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'claude',
      env: { PATH: '/usr/bin' },
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunPresent,
    })
    expect(out).not.toBeNull()
    expect(out?.source).toBe('bundled-bun')
    expect(out.argv).toEqual([
      'env',
      `PATH=${dirname(FAKE_BUN_PATH)}:/usr/bin`,
      FAKE_BUN_PATH,
      'x',
      '--bun',
      '--silent',
      '--package',
      HOST_ACP_ADAPTER_CONFIG.claude.acpPackageSpec,
      HOST_ACP_ADAPTER_CONFIG.claude.acpBin,
    ])
  })

  it('returns the bundled-bun launcher for codex when the binary exists', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      env: { PATH: '/usr/bin' },
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunPresent,
    })
    expect(out?.source).toBe('bundled-bun')
    expect(out?.argv).toEqual([
      'env',
      `PATH=${dirname(FAKE_BUN_PATH)}:/usr/bin`,
      FAKE_BUN_PATH,
      'x',
      '--bun',
      '--silent',
      '--package',
      HOST_ACP_ADAPTER_CONFIG.codex.acpPackageSpec,
      HOST_ACP_ADAPTER_CONFIG.codex.acpBin,
    ])
  })

  it('passes Codex process configuration through the bundled launcher', () => {
    const codexConfig = JSON.stringify({
      developer_instructions: "Use BrowserOS, not Codex's browser.\n",
    })
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      env: { PATH: '/usr/bin' },
      resourcesDir: '/fake/resources',
      spawnEnv: {
        CODEX_CONFIG: codexConfig,
        INITIAL_AGENT_MODE: 'agent-full-access',
      },
      resolveBundledBun: stubBunPresent,
    })
    expect(out.argv).toContain(`CODEX_CONFIG=${codexConfig}`)
    expect(out.argv).toContain('INITIAL_AGENT_MODE=agent-full-access')
    expect(out.argv).toContain(`PATH=${dirname(FAKE_BUN_PATH)}:/usr/bin`)
  })

  it('falls back to the host npx command when the bundled binary is missing', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'claude',
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunMissing,
    })
    expect(out?.source).toBe('host-npx-fallback')
    expect(out?.argv).toEqual(HOST_ACP_ADAPTER_CONFIG.claude.acpArgv)
  })

  it('passes process configuration through the host npx fallback', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      resourcesDir: '/fake/resources',
      spawnEnv: { INITIAL_AGENT_MODE: 'agent-full-access' },
      resolveBundledBun: stubBunMissing,
    })
    expect(out.argv[0]).toBe('env')
    expect(out.argv).toContain('INITIAL_AGENT_MODE=agent-full-access')
    expect(out.argv).toContain('npx')
    expect(out.argv).toContain(HOST_ACP_ADAPTER_CONFIG.codex.acpPackageSpec)
  })

  it('preserves a bundled bun path with spaces', () => {
    const bunWithSpaces =
      '/Applications/BrowserOS App/Contents/bin/third party/bun'
    const out = resolveAcpSpawnCommand({
      agentType: 'claude',
      resourcesDir: '/Applications/BrowserOS.app/Contents/Resources',
      resolveBundledBun: () => bunWithSpaces,
    })
    const bunIndex = out.argv.indexOf(bunWithSpaces)
    expect(out.argv[0]).toBe('env')
    expect(bunIndex).toBeGreaterThanOrEqual(0)
    expect(out.argv.slice(bunIndex)).toEqual([
      bunWithSpaces,
      'x',
      '--bun',
      '--silent',
      '--package',
      HOST_ACP_ADAPTER_CONFIG.claude.acpPackageSpec,
      HOST_ACP_ADAPTER_CONFIG.claude.acpBin,
    ])
  })

  it('uses structured argv for the Windows bundled launcher', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      env: { Path: 'C:\\Windows\\System32' },
      resourcesDir: 'C:\\fake\\resources',
      platform: 'win32',
      spawnEnv: { INITIAL_AGENT_MODE: 'agent-full-access' },
      resolveBundledBun: () => WINDOWS_BUN_PATH,
    })

    expect(out?.source).toBe('bundled-bun')
    expect(out.argv.slice(0, 3)).toEqual([
      WINDOWS_BUN_PATH,
      '--eval',
      expect.any(String),
    ])
    const payload = decodeEnvironmentPayload(out.argv[3])
    expect(payload.argv).toEqual([
      WINDOWS_BUN_PATH,
      'x',
      '--bun',
      '--silent',
      '--package',
      HOST_ACP_ADAPTER_CONFIG.codex.acpPackageSpec,
      HOST_ACP_ADAPTER_CONFIG.codex.acpBin,
    ])
    expect(payload.env.INITIAL_AGENT_MODE).toBe('agent-full-access')
    expect(payload.env.Path).toContain('C:\\Windows\\System32')
  })

  it('uses cmd.exe for the Windows host npx fallback', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      env: {
        ComSpec: 'C:\\Windows\\System32\\cmd.exe',
        Path: 'C:\\Windows\\System32',
      },
      platform: 'win32',
      resourcesDir: 'C:\\missing',
      spawnEnv: { INITIAL_AGENT_MODE: 'agent-full-access' },
      resolveBundledBun: stubBunMissing,
    })

    expect(out.argv.slice(0, 2)).toEqual(['node', '--eval'])
    const payload = decodeEnvironmentPayload(out.argv[3])
    expect(payload.argv.slice(0, 4)).toEqual([
      'C:\\Windows\\System32\\cmd.exe',
      '/d',
      '/s',
      '/c',
    ])
    expect(payload.argv[4]).toContain('npx')
    expect(payload.argv[4]).toContain('@agentclientprotocol/codex-acp@^^1.0.2')
    expect(payload.env.INITIAL_AGENT_MODE).toBe('agent-full-access')
  })

  it('wraps Windows host npx even without extra environment', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'claude',
      env: {
        ComSpec: 'C:\\Windows\\System32\\cmd.exe',
        Path: 'C:\\Windows\\System32',
      },
      platform: 'win32',
      resolveBundledBun: stubBunMissing,
    })

    expect(out.argv.slice(0, 2)).toEqual(['node', '--eval'])
    const payload = decodeEnvironmentPayload(out.argv[3])
    expect(payload.argv[0]).toBe('C:\\Windows\\System32\\cmd.exe')
    expect(payload.argv[4]).toContain(
      '@agentclientprotocol/claude-agent-acp@^^0.31.0',
    )
  })

  it('does not replace the user Codex home', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunPresent,
    })
    expect(out?.argv.join('\n')).not.toContain('CODEX_HOME')
  })
})

describe('resolveAcpSpawnCommand (custom)', () => {
  const noLoginPath = () => undefined

  it('runs a custom command as given, split into argv', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'custom',
      customCommand: 'npx -y @scope/my-agent-acp --stdio',
      platform: 'darwin',
      resolveLoginShellPath: noLoginPath,
    })
    expect(out.source).toBe('custom')
    expect(out.argv).toEqual(['npx', '-y', '@scope/my-agent-acp', '--stdio'])
  })

  it('injects custom env at the process-launch boundary', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'custom',
      customCommand: 'my-agent',
      spawnEnv: { MY_AGENT_KEY: 'secret' },
      platform: 'darwin',
      resolveLoginShellPath: noLoginPath,
    })
    expect(out.source).toBe('custom')
    expect(out.argv).toEqual(['env', 'MY_AGENT_KEY=secret', 'my-agent'])
  })

  it('does not wrap the custom command in bundled-bun', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'custom',
      customCommand: 'my-agent --stdio',
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunPresent,
      platform: 'darwin',
      resolveLoginShellPath: noLoginPath,
    })
    expect(out.argv).toEqual(['my-agent', '--stdio'])
    expect(out.argv.join(' ')).not.toContain('--package')
  })

  it('prepends the login-shell PATH so profile-installed binaries resolve', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'custom',
      customCommand: 'opencode acp',
      env: { PATH: '/usr/bin' },
      platform: 'darwin',
      resolveLoginShellPath: () => '/opt/homebrew/bin:/usr/bin',
    })
    expect(out.argv).toEqual([
      'env',
      'PATH=/opt/homebrew/bin:/usr/bin',
      'opencode',
      'acp',
    ])
  })
})
