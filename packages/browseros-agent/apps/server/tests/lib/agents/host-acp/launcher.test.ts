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

function splitCommandLikeAcpx(value: string): {
  command: string
  args: string[]
} {
  const parts: string[] = []
  let current = ''
  let quote: string | null = null
  let escaping = false

  for (const ch of value) {
    if (escaping) {
      current += ch
      escaping = false
      continue
    }
    if (ch === '\\' && quote !== "'") {
      escaping = true
      continue
    }
    if (quote) {
      if (ch === quote) {
        quote = null
      } else {
        current += ch
      }
      continue
    }
    if (ch === "'" || ch === '"') {
      quote = ch
      continue
    }
    if (/\s/.test(ch)) {
      if (current.length > 0) {
        parts.push(current)
        current = ''
      }
      continue
    }
    current += ch
  }

  if (escaping) current += '\\'
  if (current.length > 0) parts.push(current)
  return { command: parts[0] ?? '', args: parts.slice(1) }
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
    expect(out?.command).toBe(
      `env PATH='${dirname(FAKE_BUN_PATH)}:/usr/bin' '${FAKE_BUN_PATH}' x --bun --silent --package '${HOST_ACP_ADAPTER_CONFIG.claude.acpPackageSpec}' '${HOST_ACP_ADAPTER_CONFIG.claude.acpBin}'`,
    )
  })

  it('returns the bundled-bun launcher for codex when the binary exists', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      env: { PATH: '/usr/bin' },
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunPresent,
    })
    expect(out?.source).toBe('bundled-bun')
    expect(out?.command).toBe(
      `env PATH='${dirname(FAKE_BUN_PATH)}:/usr/bin' '${FAKE_BUN_PATH}' x --bun --silent --package '${HOST_ACP_ADAPTER_CONFIG.codex.acpPackageSpec}' '${HOST_ACP_ADAPTER_CONFIG.codex.acpBin}'`,
    )
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
    const split = splitCommandLikeAcpx(out.command)

    expect(split.command).toBe('env')
    expect(split.args).toContain(`CODEX_CONFIG=${codexConfig}`)
    expect(split.args).toContain('INITIAL_AGENT_MODE=agent-full-access')
    expect(split.args).toContain(`PATH=${dirname(FAKE_BUN_PATH)}:/usr/bin`)
  })

  it('falls back to the host npx command when the bundled binary is missing', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'claude',
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunMissing,
    })
    expect(out?.source).toBe('host-npx-fallback')
    expect(out?.command).toBe(HOST_ACP_ADAPTER_CONFIG.claude.acpCommand)
  })

  it('passes process configuration through the host npx fallback', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      resourcesDir: '/fake/resources',
      spawnEnv: { INITIAL_AGENT_MODE: 'agent-full-access' },
      resolveBundledBun: stubBunMissing,
    })
    const split = splitCommandLikeAcpx(out.command)

    expect(split.command).toBe('env')
    expect(split.args).toContain('INITIAL_AGENT_MODE=agent-full-access')
    expect(split.args).toContain('npx')
    expect(split.args).toContain(HOST_ACP_ADAPTER_CONFIG.codex.acpPackageSpec)
  })

  it('quotes the bundled bun path so paths with spaces survive', () => {
    const bunWithSpaces =
      '/Applications/BrowserOS App/Contents/bin/third party/bun'
    const out = resolveAcpSpawnCommand({
      agentType: 'claude',
      resourcesDir: '/Applications/BrowserOS.app/Contents/Resources',
      resolveBundledBun: () => bunWithSpaces,
    })
    const split = splitCommandLikeAcpx(out?.command ?? '')
    const bunIndex = split.args.indexOf(bunWithSpaces)
    expect(split.command).toBe('env')
    expect(bunIndex).toBeGreaterThanOrEqual(0)
    expect(split.args.slice(bunIndex)).toEqual([
      bunWithSpaces,
      'x',
      '--bun',
      '--silent',
      '--package',
      HOST_ACP_ADAPTER_CONFIG.claude.acpPackageSpec,
      HOST_ACP_ADAPTER_CONFIG.claude.acpBin,
    ])
  })

  it('preserves Windows bundled bun path separators through acpx command splitting', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'claude',
      resourcesDir: 'C:\\fake\\resources',
      platform: 'win32',
      resolveBundledBun: () => WINDOWS_BUN_PATH,
    })

    expect(out?.source).toBe('bundled-bun')
    const split = splitCommandLikeAcpx(out?.command ?? '')
    expect(split.args).toContain(WINDOWS_BUN_PATH)
  })

  it('does not replace the user Codex home', () => {
    const out = resolveAcpSpawnCommand({
      agentType: 'codex',
      resourcesDir: '/fake/resources',
      resolveBundledBun: stubBunPresent,
    })
    expect(out?.command).not.toContain('CODEX_HOME')
  })
})
