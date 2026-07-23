import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { generateEnvExample } from './generate'
import { ENV_REGISTRY } from './registry'

const ROOT_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '../../../..')

describe('ENV_REGISTRY', () => {
  test('contains the root env consolidation key census in order', () => {
    expect(ENV_REGISTRY.map((spec) => spec.key)).toEqual([
      'CDP_PROTOCOL_JSON',
      'BROWSEROS_BINARY',
      'BROWSEROS_CDP_PORT',
      'BROWSEROS_SERVER_PORT',
      'BROWSEROS_EXTENSION_PORT',
      'VITE_PUBLIC_POSTHOG_KEY',
      'VITE_PUBLIC_POSTHOG_HOST',
      'VITE_PUBLIC_SENTRY_DSN',
      'VITE_PUBLIC_BROWSEROS_API',
      'VITE_ALPHA_FEATURES',
      'GRAPHQL_SCHEMA_PATH',
      'SENTRY_AUTH_TOKEN',
      'SENTRY_ORG',
      'SENTRY_PROJECT',
      'VITE_BROWSEROS_CLAW_API_URL',
      'BROWSEROS_USER_DATA_DIR',
      'BROWSEROS_CLAW_CDP_PORT',
      'BROWSERCLAW_DIR',
      'CLAW_POSTHOG_KEY',
      'CLAW_POSTHOG_HOST',
      'VITE_CLAW_POSTHOG_KEY',
      'VITE_CLAW_POSTHOG_HOST',
      'BROWSEROS_CONFIG_URL',
      'BROWSEROS_TRUSTED_ORIGINS',
      'POSTHOG_API_KEY',
      'SENTRY_DSN',
      'NODE_ENV',
      'LOG_LEVEL',
      'BROWSEROS_AI_SDK_DEVTOOLS',
      'BROWSEROS_TEST_HEADLESS',
      'R2_ACCOUNT_ID',
      'R2_ACCESS_KEY_ID',
      'R2_SECRET_ACCESS_KEY',
      'R2_BUCKET',
      'ESIGNER_USERNAME',
      'ESIGNER_PASSWORD',
      'ESIGNER_TOTP_SECRET',
      'ESIGNER_CREDENTIAL_ID',
    ])
    expect(ENV_REGISTRY.map((spec) => spec.key)).not.toContain(
      'R2_UPLOAD_PREFIX',
    )
    expect(ENV_REGISTRY.map((spec) => spec.key)).not.toContain(
      'R2_DOWNLOAD_PREFIX',
    )
  })

  test('registers the Claw analytics production contract and eSigner inputs', () => {
    const specs = Object.fromEntries(
      ENV_REGISTRY.map((spec) => [spec.key, spec]),
    )

    expect(specs.CLAW_POSTHOG_KEY).toMatchObject({
      section: 'claw',
      secret: true,
      modes: {
        development: { value: '' },
        production: { value: '' },
      },
    })
    expect(specs.CLAW_POSTHOG_HOST).toMatchObject({
      section: 'claw',
      secret: false,
      modes: {
        development: {
          value: 'https://us.i.posthog.com',
          commented: true,
        },
        production: {
          value: 'https://us.i.posthog.com',
          commented: true,
        },
      },
    })
    expect(specs.VITE_CLAW_POSTHOG_KEY).toMatchObject({
      section: 'claw',
      secret: true,
      modes: {
        development: { value: '' },
        production: { value: '' },
      },
    })
    expect(specs.VITE_CLAW_POSTHOG_KEY.description).toContain(
      'embedded in the shipped client bundle',
    )
    expect(specs.VITE_CLAW_POSTHOG_HOST).toMatchObject({
      section: 'claw',
      secret: false,
      modes: {
        development: {
          value: 'https://us.i.posthog.com',
          commented: true,
        },
        production: {
          value: 'https://us.i.posthog.com',
          commented: true,
        },
      },
    })

    expect(specs.ESIGNER_USERNAME).toMatchObject({
      section: 'sign',
      secret: false,
      modes: { production: { value: '' } },
    })
    expect(specs.ESIGNER_PASSWORD).toMatchObject({
      section: 'sign',
      secret: true,
      modes: { production: { value: '' } },
    })
    expect(specs.ESIGNER_TOTP_SECRET).toMatchObject({
      section: 'sign',
      secret: true,
      modes: { production: { value: '' } },
    })
    expect(specs.ESIGNER_CREDENTIAL_ID).toMatchObject({
      section: 'sign',
      secret: false,
      modes: { production: { value: '' } },
    })
  })
})

describe('generateEnvExample', () => {
  test('is deterministic', () => {
    expect(generateEnvExample('development')).toBe(
      generateEnvExample('development'),
    )
    expect(generateEnvExample('production')).toBe(
      generateEnvExample('production'),
    )
  })

  test('renders Claw analytics by mode and eSigner inputs for production', () => {
    const development = generateEnvExample('development')
    const production = generateEnvExample('production')

    expect(development).toContain('\nCLAW_POSTHOG_KEY=\n')
    expect(development).toContain(
      '\n# CLAW_POSTHOG_HOST=https://us.i.posthog.com\n',
    )
    expect(development).toContain('\nVITE_CLAW_POSTHOG_KEY=\n')
    expect(development).toContain(
      '\n# VITE_CLAW_POSTHOG_HOST=https://us.i.posthog.com\n',
    )

    expect(production).toContain('\nCLAW_POSTHOG_KEY=\n')
    expect(production).toContain(
      '\n# CLAW_POSTHOG_HOST=https://us.i.posthog.com\n',
    )
    expect(production).toContain('\nVITE_CLAW_POSTHOG_KEY=\n')
    expect(production).toContain(
      '\n# VITE_CLAW_POSTHOG_HOST=https://us.i.posthog.com\n',
    )
    expect(production).toContain('\n# --- sign ---\n')
    expect(production).toContain('\nESIGNER_USERNAME=\n')
    expect(production).toContain('\nESIGNER_PASSWORD=\n')
    expect(production).toContain('\nESIGNER_TOTP_SECRET=\n')
    expect(production).toContain('\nESIGNER_CREDENTIAL_ID=\n')
    expect(production.indexOf('# --- sign ---')).toBeGreaterThan(
      production.indexOf('# --- upload ---'),
    )
  })

  test.each([
    ['development', '.env.development.example'],
    ['production', '.env.production.example'],
  ] as const)('matches committed %s example', (mode, file) => {
    expect(generateEnvExample(mode)).toBe(
      readFileSync(join(ROOT_DIR, file), 'utf8'),
    )
  })
})
