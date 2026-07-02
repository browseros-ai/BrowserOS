import { describe, expect, test } from 'bun:test'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { loadClawConfig, resolveDefaultResourcesDir } from '../src/config'
import {
  CLAW_API_PORT_DEFAULT,
  CLAW_CDP_PORT_DEFAULT,
} from '../src/shared/port'

async function writeConfig(contents: unknown): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'claw-config-'))
  const path = join(dir, 'config.json')
  await writeFile(
    path,
    typeof contents === 'string' ? contents : JSON.stringify(contents),
  )
  return path
}

describe('loadClawConfig', () => {
  test('resolves default resources next to the packaged executable', () => {
    expect(
      resolveDefaultResourcesDir('/service/workdir', {
        execPath: '/opt/browseros-claw/resources/bin/browseros-claw-server',
        isStandalone: true,
      }),
    ).toBe('/opt/browseros-claw/resources')
  })

  test('loads the checked-in sample config', () => {
    const samplePath = join(import.meta.dir, '..', 'config.sample.json')
    const result = loadClawConfig({
      argv: ['--config', samplePath],
      cwd: '/',
      env: {},
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: CLAW_API_PORT_DEFAULT,
        proxyPort: CLAW_API_PORT_DEFAULT,
        cdpPort: CLAW_CDP_PORT_DEFAULT,
        resourcesDir: join(dirname(samplePath), 'resources'),
      },
    })
  })

  test('requires --config instead of defaulting from source runs or env', async () => {
    const envConfigPath = await writeConfig({
      ports: {
        server: 9420,
        cdp: 9020,
      },
    })

    const result = loadClawConfig({
      argv: [],
      cwd: '/',
      env: {
        CLAW_SERVER_PORT: '9420',
        BROWSEROS_CLAW_CDP_PORT: '9020',
        CLAW_CONFIG: envConfigPath,
      },
    })

    expect(result).toEqual({
      ok: false,
      error: '--config is required',
    })
  })

  test('maps shared sidecar ports and resources into ClawConfig', async () => {
    const configPath = await writeConfig({
      ports: {
        server: 9420,
        cdp: 9020,
        proxy: 9120,
      },
      directories: {
        resources: '../resources',
        execution: '../execution',
      },
      flags: {
        allow_remote_in_mcp: true,
      },
      instance: {
        client_id: 'client-123',
        install_id: 'install-456',
      },
    })

    const result = loadClawConfig({
      argv: ['bun', 'src/main.ts', '--config', configPath],
      cwd: '/',
      env: {},
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9420,
        proxyPort: 9120,
        cdpPort: 9020,
        resourcesDir: join(dirname(configPath), '../resources'),
      },
    })
  })

  test('accepts standalone binary argv without a script path', async () => {
    const configPath = await writeConfig({
      ports: {
        server: 9420,
        cdp: 9020,
      },
    })

    const result = loadClawConfig({
      argv: ['/usr/bin/browseros-claw-server', '--config', configPath],
      cwd: '/',
      env: {},
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9420,
        proxyPort: undefined,
        cdpPort: 9020,
        resourcesDir: '/resources',
      },
    })
  })

  test('falls back to default resources when the sidecar omits directories.resources', async () => {
    const configPath = await writeConfig({
      ports: {
        server: 9420,
        cdp: 9020,
      },
    })

    const result = loadClawConfig({
      argv: ['--config', configPath],
      cwd: '/service/workdir',
      env: {},
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9420,
        proxyPort: undefined,
        cdpPort: 9020,
        resourcesDir: '/service/workdir/resources',
      },
    })
  })

  test('tolerates unknown future and BrowserOS-only sidecar fields', async () => {
    const configPath = await writeConfig({
      future_top_level: true,
      ports: {
        server: '9420',
        cdp: '9020',
        proxy: 9120,
        future_port: 1234,
      },
      directories: {
        resources: 'resources',
        execution: 'execution',
        cache: 'cache',
      },
      flags: {
        allow_remote_in_mcp: false,
        future_flag: true,
      },
      instance: {
        client_id: 'client-123',
        browseros_version: '1.2.3',
      },
    })

    const result = loadClawConfig({
      argv: ['--config', configPath],
      cwd: '/',
      env: {},
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9420,
        proxyPort: 9120,
        cdpPort: 9020,
        resourcesDir: join(dirname(configPath), 'resources'),
      },
    })
  })

  test('rejects old Claw config inputs', async () => {
    const configPath = await writeConfig({
      ports: {
        server: 9420,
        cdp: 9020,
      },
    })

    const resourcesResult = loadClawConfig({
      argv: ['--config', configPath, '--resources-dir', 'cli-resources'],
      cwd: '/',
      env: {},
    })
    expect(resourcesResult.ok).toBe(false)
    if (!resourcesResult.ok) {
      expect(resourcesResult.error).toContain('unknown option')
      expect(resourcesResult.error).toContain('--resources-dir')
    }

    const envResult = loadClawConfig({
      argv: ['--config', configPath],
      cwd: '/',
      env: {
        CLAW_SERVER_PORT: '1',
        BROWSEROS_CLAW_CDP_PORT: '2',
        BROWSEROS_CLAW_RESOURCES_DIR: 'env-resources',
      },
    })
    expect(envResult).toEqual({
      ok: true,
      value: {
        port: 9420,
        proxyPort: undefined,
        cdpPort: 9020,
        resourcesDir: '/resources',
      },
    })
  })

  test('rejects an explicitly empty --config value', () => {
    const result = loadClawConfig({
      argv: ['--config='],
      cwd: '/',
      env: {},
    })

    expect(result).toEqual({
      ok: false,
      error: '--config requires a path',
    })
  })

  test('returns clear errors for missing required sidecar fields', async () => {
    const configPath = await writeConfig({
      ports: {
        cdp: 9020,
        http_mcp: 9420,
        extension: 9300,
      },
    })

    const result = loadClawConfig({
      argv: ['--config', configPath],
      cwd: '/',
      env: {},
    })

    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toContain('ports.server')
      expect(result.error).not.toContain('http_mcp')
    }
  })

  test('returns shared parser errors for invalid known JSON ports', async () => {
    const configPath = await writeConfig({
      ports: {
        server: 70000,
        cdp: 9020,
      },
    })

    const result = loadClawConfig({
      argv: ['--config', configPath],
      cwd: '/',
      env: {},
    })

    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toContain('ports.server')
      expect(result.error).toContain('integer port between 1 and 65535')
    }
  })

  test('returns clear errors for malformed and missing config files', async () => {
    const malformedPath = await writeConfig(`
{
  "ports": {
    "server": 9200,
  }
}
`)

    const malformed = loadClawConfig({
      argv: ['--config', malformedPath],
      cwd: '/',
      env: {},
    })
    expect(malformed.ok).toBe(false)
    if (!malformed.ok) {
      expect(malformed.error).toContain('Sidecar config file error')
      expect(malformed.error).toContain('JSON')
    }

    const missing = loadClawConfig({
      argv: ['--config', '/missing/claw.json'],
      cwd: '/',
      env: {},
    })
    expect(missing).toEqual({
      ok: false,
      error: 'Sidecar config file not found: /missing/claw.json',
    })
  })
})
