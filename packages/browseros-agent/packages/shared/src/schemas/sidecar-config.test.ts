import { describe, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { parseSidecarConfigFile } from './sidecar-config'

async function writeConfig(
  contents: unknown,
): Promise<{ dir: string; path: string }> {
  const dir = await mkdtemp(join(tmpdir(), 'sidecar-config-'))
  const path = join(dir, 'config.json')
  await writeFile(
    path,
    typeof contents === 'string' ? contents : JSON.stringify(contents),
  )
  return { dir, path }
}

describe('parseSidecarConfigFile', () => {
  test('parses Chromium-style sidecar config fields', async () => {
    const { dir, path } = await writeConfig({
      ports: {
        server: 9100,
        cdp: 9000,
        proxy: 9100,
      },
      directories: {
        resources: './resources',
        execution: './out',
      },
      flags: {
        allow_remote_in_mcp: true,
      },
      instance: {
        client_id: 'client-123',
        install_id: 'install-456',
        browseros_version: '1.2.3',
        chromium_version: '140.0.0.0',
      },
    })

    const result = parseSidecarConfigFile(path)

    expect(result).toEqual({
      ok: true,
      value: {
        ports: {
          server: 9100,
          cdp: 9000,
          proxy: 9100,
        },
        directories: {
          resources: join(dir, 'resources'),
          execution: join(dir, 'out'),
        },
        flags: {
          allow_remote_in_mcp: true,
        },
        instance: {
          client_id: 'client-123',
          install_id: 'install-456',
          browseros_version: '1.2.3',
          chromium_version: '140.0.0.0',
        },
      },
    })

    await rm(dir, { recursive: true, force: true })
  })

  test('resolves relative directories from the config file directory', async () => {
    const { dir, path } = await writeConfig({
      directories: {
        resources: '../resources',
        execution: 'logs',
      },
    })

    const result = parseSidecarConfigFile(path)

    expect(result.ok).toBe(true)
    if (result.ok) {
      expect(result.value.directories.resources).toBe(join(dir, '../resources'))
      expect(result.value.directories.execution).toBe(join(dir, 'logs'))
    }

    await rm(dir, { recursive: true, force: true })
  })

  test('parses replay retention configuration', async () => {
    const { dir, path } = await writeConfig({
      replay: { retentionDays: 14 },
    })

    const result = parseSidecarConfigFile(path)

    expect(result).toEqual({
      ok: true,
      value: {
        ports: {},
        directories: {},
        flags: {},
        instance: {},
        replay: { retentionDays: 14 },
      },
    })

    await rm(dir, { recursive: true, force: true })
  })

  test.each([
    ['ports.server', { ports: { server: 'not-a-number' } }],
    ['ports.cdp', { ports: { cdp: 1.5 } }],
    ['ports.proxy', { ports: { proxy: 0 } }],
    ['ports.server', { ports: { server: 65536 } }],
    ['ports.cdp', { ports: { cdp: '9000.5' } }],
  ])('rejects invalid known port values at %s', async (field, config) => {
    const { dir, path } = await writeConfig(config)

    const result = parseSidecarConfigFile(path)

    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toContain(field)
      expect(result.error).toContain('integer port between 1 and 65535')
    }

    await rm(dir, { recursive: true, force: true })
  })

  test('tolerates unknown top-level and nested keys without returning them', async () => {
    const { dir, path } = await writeConfig({
      future_top_level: true,
      ports: {
        server: '9100',
        future_port: 1234,
      },
      directories: {
        resources: './resources',
        cache: './cache',
      },
      flags: {
        allow_remote_in_mcp: false,
        future_flag: true,
      },
      instance: {
        client_id: 'client-123',
        future_instance: 'value',
      },
    })

    const result = parseSidecarConfigFile(path)

    expect(result.ok).toBe(true)
    if (result.ok) {
      expect(result.value).toEqual({
        ports: { server: 9100 },
        directories: { resources: join(dir, 'resources') },
        flags: { allow_remote_in_mcp: false },
        instance: { client_id: 'client-123' },
      })
      expect('future_top_level' in result.value).toBe(false)
      expect('future_port' in result.value.ports).toBe(false)
      expect('cache' in result.value.directories).toBe(false)
      expect('future_flag' in result.value.flags).toBe(false)
      expect('future_instance' in result.value.instance).toBe(false)
    }

    await rm(dir, { recursive: true, force: true })
  })

  test('does not treat legacy port aliases as known fields', async () => {
    const { dir, path } = await writeConfig({
      ports: {
        cdp: 9000,
        http_mcp: 9100,
        extension: 9300,
      },
    })

    const result = parseSidecarConfigFile(path)

    expect(result.ok).toBe(true)
    if (result.ok) {
      expect(result.value.ports).toEqual({ cdp: 9000 })
      expect('http_mcp' in result.value.ports).toBe(false)
      expect('extension' in result.value.ports).toBe(false)
    }

    await rm(dir, { recursive: true, force: true })
  })
})
