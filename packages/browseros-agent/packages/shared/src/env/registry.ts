import { z } from 'zod'

export type EnvMode = 'development' | 'production'
export type EnvSection =
  | 'dev-tools'
  | 'app'
  | 'claw'
  | 'server'
  | 'build'
  | 'upload'

export interface EnvExampleEntry {
  value: string
  commented?: boolean
}

export interface EnvKeySpec {
  key: string
  section: EnvSection
  description: string
  secret: boolean
  schema: z.ZodType<string>
  modes: Partial<Record<EnvMode, EnvExampleEntry>>
}

const stringSchema = z.string()
const urlSchema = z.string().url()
const portSchema = z.string().refine((value) => {
  if (!/^\d+$/.test(value)) {
    return false
  }
  const port = Number(value)
  return port >= 1 && port <= 65535
}, 'integer port between 1 and 65535')

export const ENV_REGISTRY: readonly EnvKeySpec[] = [
  {
    key: 'CDP_PROTOCOL_JSON',
    section: 'dev-tools',
    description: 'CDP protocol JSON input for browser protocol codegen.',
    secret: false,
    schema: stringSchema,
    modes: {
      development: {
        value:
          '/path/to/chromium/src/out/Default_arm64/gen/third_party/blink/public/devtools_protocol/protocol.json',
        commented: true,
      },
    },
  },
  {
    key: 'BROWSEROS_BINARY',
    section: 'dev-tools',
    description: 'BrowserOS binary used by app and Claw development.',
    secret: false,
    schema: stringSchema,
    modes: {
      development: {
        value: '/Applications/BrowserOS.app/Contents/MacOS/BrowserOS',
      },
    },
  },
  {
    key: 'BROWSEROS_CDP_PORT',
    section: 'app',
    description: 'Chromium remote debugging port for app development.',
    secret: false,
    schema: portSchema,
    modes: { development: { value: '9005' } },
  },
  {
    key: 'BROWSEROS_SERVER_PORT',
    section: 'app',
    description:
      'BrowserOS server port, also used as the Claw browser launch port.',
    secret: false,
    schema: portSchema,
    modes: { development: { value: '9105' } },
  },
  {
    key: 'BROWSEROS_EXTENSION_PORT',
    section: 'app',
    description: 'Extension dev server port for app development.',
    secret: false,
    schema: portSchema,
    modes: { development: { value: '9305' } },
  },
  {
    key: 'VITE_PUBLIC_POSTHOG_KEY',
    section: 'app',
    description: 'Browser bundle PostHog key.',
    secret: true,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'VITE_PUBLIC_POSTHOG_HOST',
    section: 'app',
    description: 'Browser bundle PostHog host.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'VITE_PUBLIC_SENTRY_DSN',
    section: 'app',
    description: 'Browser bundle Sentry DSN.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'VITE_PUBLIC_BROWSEROS_API',
    section: 'app',
    description: 'Public BrowserOS API URL exposed to the browser bundle.',
    secret: false,
    schema: urlSchema,
    modes: { development: { value: 'https://api.browseros.com' } },
  },
  {
    key: 'VITE_ALPHA_FEATURES',
    section: 'app',
    description: 'Alpha feature flag for the browser bundle.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: 'true' } },
  },
  {
    key: 'GRAPHQL_SCHEMA_PATH',
    section: 'app',
    description:
      'Optional GraphQL schema path; falls back to schema/schema.graphql.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'SENTRY_AUTH_TOKEN',
    section: 'app',
    description: 'Sentry auth token for source-map uploads.',
    secret: true,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'SENTRY_ORG',
    section: 'app',
    description: 'Sentry organization for source-map uploads.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'SENTRY_PROJECT',
    section: 'app',
    description: 'Sentry project for source-map uploads.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'VITE_BROWSEROS_CLAW_API_URL',
    section: 'claw',
    description:
      'Optional Claw API base URL override; tools/dev injects this in dev:claw flows and real env wins.',
    secret: false,
    schema: stringSchema,
    modes: {
      development: { value: 'http://127.0.0.1:9200', commented: true },
    },
  },
  {
    key: 'BROWSEROS_USER_DATA_DIR',
    section: 'claw',
    description:
      'Optional Chromium user data directory override for Claw dev browser launches.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '/tmp/my-browseros-dev', commented: true } },
  },
  {
    key: 'BROWSEROS_CLAW_CDP_PORT',
    section: 'claw',
    description:
      'Optional Chromium remote debugging port override shared by Claw app and server dev.',
    secret: false,
    schema: portSchema,
    modes: { development: { value: '49337', commented: true } },
  },
  {
    key: 'BROWSERCLAW_DIR',
    section: 'claw',
    description: 'Optional BrowserClaw state root override.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '~/.browserclaw-dev', commented: true } },
  },
  {
    key: 'BROWSEROS_CONFIG_URL',
    section: 'server',
    description: 'BrowserOS server config URL required by production builds.',
    secret: false,
    schema: urlSchema,
    modes: {
      development: {
        value: 'https://llm.browseros.com/api/browseros-server/config',
      },
      production: {
        value: 'https://llm.browseros.com/api/browseros-server/config',
      },
    },
  },
  {
    key: 'BROWSEROS_TRUSTED_ORIGINS',
    section: 'server',
    description: 'Trusted origins for local server development.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: '' } },
  },
  {
    key: 'POSTHOG_API_KEY',
    section: 'server',
    description: 'Server telemetry key; CLI release builds read the same key.',
    secret: true,
    schema: stringSchema,
    modes: { development: { value: '' }, production: { value: '' } },
  },
  {
    key: 'SENTRY_DSN',
    section: 'server',
    description: 'Server Sentry DSN.',
    secret: false,
    schema: stringSchema,
    modes: {
      development: { value: '' },
      production: { value: '' },
    },
  },
  {
    key: 'NODE_ENV',
    section: 'server',
    description: 'Node environment for server and build scripts.',
    secret: false,
    schema: stringSchema,
    modes: {
      development: { value: 'development' },
      production: { value: 'production' },
    },
  },
  {
    key: 'LOG_LEVEL',
    section: 'server',
    description: 'Server log level.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: 'info' }, production: { value: 'info' } },
  },
  {
    key: 'BROWSEROS_AI_SDK_DEVTOOLS',
    section: 'server',
    description:
      'Optional AI SDK DevTools capture toggle for local server runs.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: 'true', commented: true } },
  },
  {
    key: 'BROWSEROS_TEST_HEADLESS',
    section: 'server',
    description: 'Headless browser setting for local server tests.',
    secret: false,
    schema: stringSchema,
    modes: { development: { value: 'true' } },
  },
  {
    key: 'AGENT_RUNNER_JWT_SECRET',
    section: 'build',
    description:
      'Agent runner JWT secret inlined into server builds when present.',
    secret: true,
    schema: stringSchema,
    modes: { production: { value: '' } },
  },
  {
    key: 'R2_ACCOUNT_ID',
    section: 'upload',
    description: 'R2 account ID for production artifact uploads.',
    secret: true,
    schema: stringSchema,
    modes: { production: { value: '' } },
  },
  {
    key: 'R2_ACCESS_KEY_ID',
    section: 'upload',
    description: 'R2 access key ID for production artifact uploads.',
    secret: true,
    schema: stringSchema,
    modes: { production: { value: '' } },
  },
  {
    key: 'R2_SECRET_ACCESS_KEY',
    section: 'upload',
    description: 'R2 secret access key for production artifact uploads.',
    secret: true,
    schema: stringSchema,
    modes: { production: { value: '' } },
  },
  {
    key: 'R2_BUCKET',
    section: 'upload',
    description: 'R2 bucket for production artifact uploads.',
    secret: false,
    schema: stringSchema,
    modes: { production: { value: 'browseros' } },
  },
]

/** Finds a registry entry for callers that validate or report individual keys. */
export function findEnvKeySpec(key: string): EnvKeySpec | undefined {
  return ENV_REGISTRY.find((spec) => spec.key === key)
}
