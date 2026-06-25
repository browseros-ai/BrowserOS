import { describe, expect, test } from 'bun:test'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { loadClawConfig } from '../src/config'
import {
  CLAW_API_PORT_DEFAULT,
  CLAW_CDP_PORT_DEFAULT,
} from '../src/shared/port'

async function writeConfig(contents: string): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'claw-config-'))
  const path = join(dir, 'config.yaml')
  await writeFile(path, contents)
  return path
}

describe('loadClawConfig', () => {
  test('uses standalone defaults when no env or config file is provided', () => {
    const result = loadClawConfig({ argv: [], env: {}, cwd: '/' })

    expect(result).toEqual({
      ok: true,
      value: {
        port: CLAW_API_PORT_DEFAULT,
        cdpPort: CLAW_CDP_PORT_DEFAULT,
      },
    })
  })

  test('reads Claw-specific port env values', () => {
    const result = loadClawConfig({
      argv: [],
      cwd: '/',
      env: {
        CLAW_SERVER_PORT: '9310',
        BROWSEROS_CLAW_CDP_PORT: '9010',
      },
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9310,
        cdpPort: 9010,
      },
    })
  })

  test('reads ports from a YAML config file', async () => {
    const configPath = await writeConfig(`
ports:
  server: 9420
  cdp: 9020
`)

    const result = loadClawConfig({
      argv: [],
      cwd: '/',
      env: { CLAW_CONFIG: configPath },
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9420,
        cdpPort: 9020,
      },
    })
  })

  test('lets env ports override YAML config values', async () => {
    const configPath = await writeConfig(`
ports:
  server: 9520
  cdp: 9120
`)

    const result = loadClawConfig({
      argv: [],
      cwd: '/',
      env: {
        CLAW_SERVER_PORT: '9310',
        BROWSEROS_CLAW_CDP_PORT: '9010',
        CLAW_CONFIG: configPath,
      },
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9310,
        cdpPort: 9010,
      },
    })
  })

  test('--config wins over CLAW_CONFIG for the config file path', async () => {
    const envConfigPath = await writeConfig(`
ports:
  server: 9300
  cdp: 9000
`)
    const cliConfigPath = await writeConfig(`
ports:
  server: 9600
  cdp: 9200
`)

    const result = loadClawConfig({
      argv: ['bun', 'src/main.ts', '--config', cliConfigPath],
      cwd: '/',
      env: { CLAW_CONFIG: envConfigPath },
    })

    expect(result).toEqual({
      ok: true,
      value: {
        port: 9600,
        cdpPort: 9200,
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

  test('rejects a blank --config value instead of falling back to CLAW_CONFIG', async () => {
    const envConfigPath = await writeConfig(`
ports:
  server: 9300
`)

    const result = loadClawConfig({
      argv: ['--config', '   '],
      cwd: '/',
      env: { CLAW_CONFIG: envConfigPath },
    })

    expect(result).toEqual({
      ok: false,
      error: '--config requires a path',
    })
  })

  test('returns a clear error for invalid env ports', () => {
    const result = loadClawConfig({
      argv: [],
      cwd: '/',
      env: { CLAW_SERVER_PORT: 'abc' },
    })

    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toContain('CLAW_SERVER_PORT')
      expect(result.error).toContain('integer port between 1 and 65535')
    }
  })

  test('returns a clear error for invalid YAML ports', async () => {
    const configPath = await writeConfig(`
ports:
  server: 70000
`)

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

  test('returns a clear error for unknown YAML port keys', async () => {
    const configPath = await writeConfig(`
ports:
  cdpPort: 9020
`)

    const result = loadClawConfig({
      argv: ['--config', configPath],
      cwd: '/',
      env: {},
    })

    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toContain('ports')
      expect(result.error).toContain('cdpPort')
    }
  })

  test('returns a clear error for malformed YAML', async () => {
    const configPath = await writeConfig(`
ports:
  server: [9200
`)

    const result = loadClawConfig({
      argv: ['--config', configPath],
      cwd: '/',
      env: {},
    })

    expect(result.ok).toBe(false)
    if (!result.ok) {
      expect(result.error).toContain('Config file error:')
      expect(result.error).toContain('Flow sequence')
    }
  })

  test('returns a clear error for a missing config file', () => {
    const result = loadClawConfig({
      argv: ['--config', '/missing/claw.yaml'],
      cwd: '/',
      env: {},
    })

    expect(result).toEqual({
      ok: false,
      error: 'Config file not found: /missing/claw.yaml',
    })
  })
})
