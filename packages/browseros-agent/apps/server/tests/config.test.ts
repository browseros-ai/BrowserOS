import { afterEach, beforeEach, describe, it } from 'bun:test'
import assert from 'node:assert'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { loadServerConfig } from '../src/config'

describe('loadServerConfig', () => {
  let tempDir: string
  let originalEnv: NodeJS.ProcessEnv

  beforeEach(() => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'browseros-config-test-'))
    originalEnv = { ...process.env }

    delete process.env.BROWSEROS_CDP_PORT
    delete process.env.BROWSEROS_SERVER_PORT
    delete process.env.BROWSEROS_EXTENSION_PORT
    delete process.env.BROWSEROS_RESOURCES_DIR
    delete process.env.BROWSEROS_EXECUTION_DIR
    delete process.env.BROWSEROS_INSTALL_ID
    delete process.env.BROWSEROS_CLIENT_ID
    delete process.env.BROWSEROS_AI_SDK_DEVTOOLS
  })

  afterEach(() => {
    fs.rmSync(tempDir, { recursive: true, force: true })
    process.env = originalEnv
  })

  it('maps a sidecar JSON config into ServerConfig', () => {
    const configPath = writeSidecarConfig({
      ports: {
        server: 9223,
        cdp: 9222,
        proxy: 9444,
      },
      directories: {
        resources: 'resources',
        execution: 'execution',
      },
      flags: {
        allow_remote_in_mcp: true,
      },
      instance: {
        client_id: 'user-123',
        install_id: 'install-456',
        browseros_version: '1.0.0',
        chromium_version: '140.0.0.0',
      },
    })

    const result = loadServerConfig([
      'bun',
      'src/index.ts',
      '--config',
      configPath,
    ])

    assert.strictEqual(result.ok, true)
    if (!result.ok) return
    assert.strictEqual(result.value.cdpPort, 9222)
    assert.strictEqual(result.value.serverPort, 9223)
    assert.strictEqual(result.value.agentPort, 9223)
    assert.strictEqual(result.value.extensionPort, null)
    assert.strictEqual(
      result.value.resourcesDir,
      path.join(tempDir, 'resources'),
    )
    assert.strictEqual(
      result.value.executionDir,
      path.join(tempDir, 'execution'),
    )
    assert.strictEqual(result.value.mcpAllowRemote, true)
    assert.strictEqual(result.value.instanceClientId, 'user-123')
    assert.strictEqual(result.value.instanceInstallId, 'install-456')
    assert.strictEqual(result.value.instanceBrowserosVersion, '1.0.0')
    assert.strictEqual(result.value.instanceChromiumVersion, '140.0.0.0')
  })

  it('accepts standalone binary argv without a script path', () => {
    const configPath = writeSidecarConfig()

    const result = loadServerConfig([
      '/usr/bin/browseros_server',
      '--config',
      configPath,
    ])

    assert.strictEqual(result.ok, true)
    if (!result.ok) return
    assert.strictEqual(result.value.serverPort, 9100)
    assert.strictEqual(result.value.cdpPort, 9000)
  })

  it('requires --config instead of falling back to defaults or env', () => {
    process.env.BROWSEROS_CDP_PORT = '9222'
    process.env.BROWSEROS_SERVER_PORT = '9223'
    process.env.BROWSEROS_RESOURCES_DIR = tempDir
    process.env.BROWSEROS_EXECUTION_DIR = tempDir

    const result = loadServerConfig(['bun', 'src/index.ts'])

    assert.strictEqual(result.ok, false)
    if (result.ok) return
    assert.strictEqual(result.error, '--config is required')
  })

  it('rejects an empty --config value', () => {
    const result = loadServerConfig(['bun', 'src/index.ts', '--config='])

    assert.strictEqual(result.ok, false)
    if (result.ok) return
    assert.strictEqual(result.error, '--config requires a path')
  })

  it('rejects old runtime config CLI flags', () => {
    for (const flag of [
      '--server-port=9223',
      '--cdp-port=9222',
      '--http-mcp-port=9223',
      '--agent-port=9223',
      '--extension-port=9224',
      '--resources-dir=resources',
      '--execution-dir=execution',
      '--allow-remote-in-mcp',
    ]) {
      const result = loadServerConfig(['bun', 'src/index.ts', flag])

      assert.strictEqual(result.ok, false, flag)
      if (result.ok) continue
      assert.ok(result.error.includes('unknown option'), result.error)
      assert.ok(result.error.includes(flag.split('=')[0]), result.error)
    }
  })

  it('ignores old server runtime env vars when --config is present', () => {
    process.env.BROWSEROS_CDP_PORT = '1111'
    process.env.BROWSEROS_SERVER_PORT = '2222'
    process.env.BROWSEROS_EXTENSION_PORT = '3333'
    process.env.BROWSEROS_RESOURCES_DIR = '/wrong/resources'
    process.env.BROWSEROS_EXECUTION_DIR = '/wrong/execution'
    process.env.BROWSEROS_INSTALL_ID = 'wrong-install'
    process.env.BROWSEROS_CLIENT_ID = 'wrong-client'

    const configPath = writeSidecarConfig()
    const result = loadServerConfig([
      'bun',
      'src/index.ts',
      '--config',
      configPath,
    ])

    assert.strictEqual(result.ok, true)
    if (!result.ok) return
    assert.strictEqual(result.value.cdpPort, 9000)
    assert.strictEqual(result.value.serverPort, 9100)
    assert.strictEqual(result.value.extensionPort, null)
    assert.strictEqual(
      result.value.resourcesDir,
      path.join(tempDir, 'resources'),
    )
    assert.strictEqual(
      result.value.executionDir,
      path.join(tempDir, 'execution'),
    )
    assert.strictEqual(result.value.instanceClientId, 'client-123')
    assert.strictEqual(result.value.instanceInstallId, 'install-456')
  })

  it('fails when required sidecar fields are missing', () => {
    const configPath = writeRawSidecarConfig({
      ports: {
        cdp: 9000,
        http_mcp: 9100,
        extension: 9300,
      },
      directories: {
        resources: 'resources',
      },
    })

    const result = loadServerConfig([
      'bun',
      'src/index.ts',
      '--config',
      configPath,
    ])

    assert.strictEqual(result.ok, false)
    if (result.ok) return
    assert.ok(result.error.includes('ports.server'), result.error)
    assert.ok(result.error.includes('directories.execution'), result.error)
    assert.ok(!result.error.includes('http_mcp'), result.error)
  })

  it('returns sidecar parser errors for invalid known JSON fields', () => {
    const configPath = writeSidecarConfig({
      ports: {
        server: 0,
        cdp: 9000,
      },
      directories: {
        resources: 'resources',
        execution: 'execution',
      },
    })

    const result = loadServerConfig([
      'bun',
      'src/index.ts',
      '--config',
      configPath,
    ])

    assert.strictEqual(result.ok, false)
    if (result.ok) return
    assert.ok(result.error.includes('ports.server'), result.error)
    assert.ok(result.error.includes('integer port between 1 and 65535'))
  })

  it('keeps the AI SDK devtools toggle outside the sidecar contract', () => {
    process.env.BROWSEROS_AI_SDK_DEVTOOLS = 'true'
    const configPath = writeSidecarConfig()

    const result = loadServerConfig([
      'bun',
      'src/index.ts',
      '--config',
      configPath,
    ])

    assert.strictEqual(result.ok, true)
    if (!result.ok) return
    assert.strictEqual(result.value.aiSdkDevtoolsEnabled, true)
  })

  it('returns clear errors for missing and malformed config files', () => {
    const missing = loadServerConfig([
      'bun',
      'src/index.ts',
      '--config',
      '/missing/config.json',
    ])
    assert.deepStrictEqual(missing, {
      ok: false,
      error: 'Sidecar config file not found: /missing/config.json',
    })

    const malformedPath = path.join(tempDir, 'malformed.json')
    fs.writeFileSync(malformedPath, '{')
    const malformed = loadServerConfig([
      'bun',
      'src/index.ts',
      '--config',
      malformedPath,
    ])
    assert.strictEqual(malformed.ok, false)
    if (malformed.ok) return
    assert.ok(malformed.error.includes('Sidecar config file error'))
  })

  function writeSidecarConfig(overrides: Record<string, unknown> = {}): string {
    const config = deepMerge(
      {
        ports: {
          server: 9100,
          cdp: 9000,
          proxy: 9100,
        },
        directories: {
          resources: 'resources',
          execution: 'execution',
        },
        flags: {
          allow_remote_in_mcp: false,
        },
        instance: {
          client_id: 'client-123',
          install_id: 'install-456',
          browseros_version: '1.2.3',
          chromium_version: '140.0.0.0',
        },
      },
      overrides,
    )
    const configPath = path.join(tempDir, 'config.json')
    fs.writeFileSync(configPath, JSON.stringify(config))
    return configPath
  }

  function writeRawSidecarConfig(config: Record<string, unknown>): string {
    const configPath = path.join(tempDir, 'config.json')
    fs.writeFileSync(configPath, JSON.stringify(config))
    return configPath
  }
})

function deepMerge(
  base: Record<string, unknown>,
  overrides: Record<string, unknown>,
): Record<string, unknown> {
  const result = { ...base }
  for (const [key, value] of Object.entries(overrides)) {
    const current = result[key]
    if (isPlainObject(current) && isPlainObject(value)) {
      result[key] = deepMerge(current, value)
    } else {
      result[key] = value
    }
  }
  return result
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
