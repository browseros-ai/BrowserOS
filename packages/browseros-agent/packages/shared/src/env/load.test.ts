import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { loadRootEnvFile, parseEnvFile, requireEnv, resolveEnv } from './load'
import { ENV_REGISTRY } from './registry'

const TEST_ENV_KEYS = ENV_REGISTRY.map((spec) => spec.key)

describe('parseEnvFile', () => {
  test('skips comments and blanks and accepts export prefixes', () => {
    expect(
      parseEnvFile(`
# comment

export FOO=bar
BAR = baz
`),
    ).toEqual({ FOO: 'bar', BAR: 'baz' })
  })

  test('parses quoted values and Go strconv.Quote escapes', () => {
    expect(
      parseEnvFile(String.raw`
SINGLE=' literal # not a comment '
MULTILINE="line1\nline2"
QUOTES="with \"quotes\""
BACKSLASH="with \\ slash"
`),
    ).toEqual({
      SINGLE: ' literal # not a comment ',
      MULTILINE: 'line1\nline2',
      QUOTES: 'with "quotes"',
      BACKSLASH: 'with \\ slash',
    })
  })

  test('parses multiline quoted values', () => {
    expect(
      parseEnvFile(`
DOUBLE="-----BEGIN KEY-----
line1
line2
-----END KEY-----"
SINGLE='first line
second line'
`),
    ).toEqual({
      DOUBLE: '-----BEGIN KEY-----\nline1\nline2\n-----END KEY-----',
      SINGLE: 'first line\nsecond line',
    })
  })

  test('throws on unterminated quoted values', () => {
    expect(() => parseEnvFile('SECRET="unterminated\nnext line')).toThrow(
      'Unterminated quoted value for SECRET',
    )
    expect(() => parseEnvFile("TOKEN='unterminated\nnext line")).toThrow(
      'Unterminated quoted value for TOKEN',
    )
  })

  test('strips inline comments from unquoted values only', () => {
    expect(
      parseEnvFile(`
UNQUOTED=value # comment
HASH=value#not-comment
DOUBLE="value # not comment"
SINGLE='value # not comment'
`),
    ).toEqual({
      UNQUOTED: 'value',
      HASH: 'value#not-comment',
      DOUBLE: 'value # not comment',
      SINGLE: 'value # not comment',
    })
  })
})

describe('resolveEnv', () => {
  let tempRoot: string | null = null
  let originalEnv: Record<string, string | undefined>

  beforeEach(() => {
    originalEnv = {}
    for (const key of TEST_ENV_KEYS) {
      originalEnv[key] = process.env[key]
      delete process.env[key]
    }
  })

  afterEach(async () => {
    for (const key of TEST_ENV_KEYS) {
      const value = originalEnv[key]
      if (value === undefined) {
        delete process.env[key]
      } else {
        process.env[key] = value
      }
    }

    if (tempRoot) {
      await rm(tempRoot, { recursive: true, force: true })
      tempRoot = null
    }
  })

  test('returns an empty file layer when the root env file is missing', async () => {
    tempRoot = await mkdtemp(join(tmpdir(), 'root-env-load-'))

    expect(loadRootEnvFile(tempRoot, 'production')).toEqual({})
    expect(
      resolveEnv({ rootDir: tempRoot, mode: 'production' }).values,
    ).not.toHaveProperty('BROWSEROS_SENTRY_DSN')
  })

  test('uses overrides before process env before the root env file', async () => {
    tempRoot = await writeRootEnv('development', { BROWSEROS_LOG: 'file' })
    process.env.BROWSEROS_LOG = 'process'

    expect(
      resolveEnv({
        rootDir: tempRoot,
        mode: 'development',
        overrides: { BROWSEROS_LOG: 'override' },
      }).values.BROWSEROS_LOG,
    ).toBe('override')

    expect(
      resolveEnv({ rootDir: tempRoot, mode: 'development' }).values
        .BROWSEROS_LOG,
    ).toBe('process')
  })

  test('demotes Bun auto-loaded values from the wrong root file', async () => {
    tempRoot = await mkdtemp(join(tmpdir(), 'root-env-load-'))
    await writeEnvFile(tempRoot, '.env.development', {
      NODE_ENV: 'development',
    })
    await writeEnvFile(tempRoot, '.env.production', { NODE_ENV: 'production' })
    process.env.NODE_ENV = 'development'

    const resolved = resolveEnv({ rootDir: tempRoot, mode: 'production' })

    expect(resolved.values.NODE_ENV).toBe('production')
    expect(resolved.sources.NODE_ENV).toBe('root-file')
    expect(resolved.demotedKeys).toEqual(['NODE_ENV'])
  })

  test('demotes wrong-source values when the target file lacks the key', async () => {
    tempRoot = await mkdtemp(join(tmpdir(), 'root-env-load-'))
    await writeEnvFile(tempRoot, '.env.development', {
      BROWSEROS_SENTRY_DSN: 'phc_dev',
    })
    await writeEnvFile(tempRoot, '.env.production', { NODE_ENV: 'production' })
    process.env.BROWSEROS_SENTRY_DSN = 'phc_dev'

    const resolved = resolveEnv({ rootDir: tempRoot, mode: 'production' })

    expect(resolved.values).not.toHaveProperty('BROWSEROS_SENTRY_DSN')
    expect(resolved.sources).not.toHaveProperty('BROWSEROS_SENTRY_DSN')
    expect(resolved.demotedKeys).toEqual(['BROWSEROS_SENTRY_DSN'])
  })

  test('keeps explicit process env when it does not match a wrong-source file', async () => {
    tempRoot = await mkdtemp(join(tmpdir(), 'root-env-load-'))
    await writeEnvFile(tempRoot, '.env.development', { BROWSEROS_LOG: 'debug' })
    await writeEnvFile(tempRoot, '.env.production', { BROWSEROS_LOG: 'info' })
    process.env.BROWSEROS_LOG = 'warn'

    expect(
      resolveEnv({ rootDir: tempRoot, mode: 'production' }).values
        .BROWSEROS_LOG,
    ).toBe('warn')
  })

  test('throws one missing-key error with section and file guidance', async () => {
    tempRoot = await writeRootEnv('production', {
      BROWSEROS_SENTRY_DSN: '   ',
    })
    const resolved = resolveEnv({ rootDir: tempRoot, mode: 'production' })

    expect(() => requireEnv(resolved, ['BROWSEROS_SENTRY_DSN'])).toThrow(
      /BROWSEROS_SENTRY_DSN \(section: server\).*\.env\.production/,
    )
  })

  test('trims returned required values and reports unknown missing keys', async () => {
    tempRoot = await writeRootEnv('development', {
      BROWSEROS_LOG: ' debug ',
    })
    const resolved = resolveEnv({ rootDir: tempRoot, mode: 'development' })

    expect(requireEnv(resolved, ['BROWSEROS_LOG'])).toEqual({
      BROWSEROS_LOG: 'debug',
    })
    expect(() => requireEnv(resolved, ['UNKNOWN_REQUIRED_KEY'])).toThrow(
      /UNKNOWN_REQUIRED_KEY\. Set it in \.env\.development/,
    )
  })

  test('does not validate unrelated ambient registry values', async () => {
    tempRoot = await writeRootEnv('production', {
      BROWSEROS_LOG: 'info',
    })
    process.env.BROWSEROS_SERVER_PORT = 'not a port'
    const resolved = resolveEnv({ rootDir: tempRoot, mode: 'production' })

    expect(requireEnv(resolved, ['BROWSEROS_LOG'])).toEqual({
      BROWSEROS_LOG: 'info',
    })
  })

  test('validates requested non-empty values only', async () => {
    tempRoot = await writeRootEnv('production', {
      BROWSEROS_SERVER_PORT: 'not a port',
    })
    const resolved = resolveEnv({ rootDir: tempRoot, mode: 'production' })

    expect(() => requireEnv(resolved, ['BROWSEROS_SERVER_PORT'])).toThrow(
      /Invalid env: BROWSEROS_SERVER_PORT \(section: app\).*port/i,
    )
  })

  async function writeRootEnv(
    mode: 'development' | 'production',
    values: Record<string, string>,
  ): Promise<string> {
    tempRoot = await mkdtemp(join(tmpdir(), 'root-env-load-'))
    await writeEnvFile(tempRoot, `.env.${mode}`, values)
    return tempRoot
  }
})

async function writeEnvFile(
  rootDir: string,
  file: string,
  values: Record<string, string>,
): Promise<void> {
  await writeFile(
    join(rootDir, file),
    `${Object.entries(values)
      .map(([key, value]) => `${key}=${value}`)
      .join('\n')}\n`,
  )
}
