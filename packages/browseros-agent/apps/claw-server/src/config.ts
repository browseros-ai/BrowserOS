import { existsSync, readFileSync } from 'node:fs'
import { isAbsolute, resolve } from 'node:path'
import { Command } from 'commander'
import { parseDocument } from 'yaml'
import { z } from 'zod'
import { CLAW_API_PORT_DEFAULT, CLAW_CDP_PORT_DEFAULT } from './shared/port'

const portSchema = z.number().int().min(1).max(65535)
const optionalPortSchema = z.preprocess(
  normalizePortInput,
  portSchema.optional(),
)
const ClawConfigSchema = z.object({
  port: portSchema,
  cdpPort: portSchema,
})
const ClawEnvSchema = z.object({
  port: optionalPortSchema,
  cdpPort: optionalPortSchema,
})
const ClawConfigFileSchema = z.object({
  ports: z
    .object({
      server: optionalPortSchema,
      cdp: optionalPortSchema,
    })
    .strict()
    .optional(),
})

export type ClawConfig = z.infer<typeof ClawConfigSchema>
export type ConfigResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: string }

interface LoadClawConfigOptions {
  argv?: string[]
  cwd?: string
  env?: Record<string, string | undefined>
}

type PartialClawConfig = Partial<ClawConfig>
type ConfigIssue = {
  path: PropertyKey[]
  message: string
  code: string
  keys?: string[]
}

/** Loads and validates Claw server ports from defaults, env, and YAML config. */
export function loadClawConfig(
  options: LoadClawConfigOptions = {},
): ConfigResult<ClawConfig> {
  const argv = options.argv ?? process.argv
  const cwd = options.cwd ?? process.cwd()
  // biome-ignore lint/style/noProcessEnv: config.ts is the sanctioned Claw config reader
  const runtimeEnv = options.env ?? process.env

  const cli = parseCliArgs(argv)
  if (!cli.ok) return cli

  const envConfig = parseRuntimeEnv(runtimeEnv)
  if (!envConfig.ok) return envConfig

  const configPath = cli.value.configPath ?? cleanString(runtimeEnv.CLAW_CONFIG)
  const fileConfig = parseConfigFile(configPath, cwd)
  if (!fileConfig.ok) return fileConfig

  const result = ClawConfigSchema.safeParse(
    mergeConfigs(getDefaults(), fileConfig.value, envConfig.value),
  )
  if (!result.success) {
    const errors = result.error.issues
      .map((issue) => `  - ${issue.path.join('.')}: ${issue.message}`)
      .join('\n')
    return {
      ok: false,
      error: `Invalid Claw server configuration:\n${errors}`,
    }
  }

  return { ok: true, value: result.data }
}

function parseCliArgs(argv: string[]): ConfigResult<{ configPath?: string }> {
  const program = new Command()

  try {
    program
      .name('claw-server')
      .description('BrowserClaw standalone API')
      .option('--config <path>', 'Path to YAML configuration file')
      .exitOverride((err) => {
        if (err.exitCode === 0) process.exit(0)
        throw err
      })
      .parse(toUserArgs(argv), { from: 'user' })
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err)
    return { ok: false, error: message }
  }

  const opts = program.opts<{ config?: string }>()
  const configPath = cleanString(opts.config)
  if (program.getOptionValueSource('config') === 'cli' && !configPath) {
    return { ok: false, error: '--config requires a path' }
  }

  return { ok: true, value: { configPath } }
}

function parseRuntimeEnv(
  env: Record<string, string | undefined>,
): ConfigResult<PartialClawConfig> {
  const result = ClawEnvSchema.safeParse({
    port: env.CLAW_SERVER_PORT,
    cdpPort: env.BROWSEROS_CLAW_CDP_PORT,
  })
  if (!result.success) {
    return {
      ok: false,
      error: `Invalid Claw server environment:\n${formatZodIssues(
        result.error.issues,
        {
          port: 'CLAW_SERVER_PORT',
          cdpPort: 'BROWSEROS_CLAW_CDP_PORT',
        },
      )}`,
    }
  }

  return {
    ok: true,
    value: omitUndefined({
      port: result.data.port,
      cdpPort: result.data.cdpPort,
    }),
  }
}

function parseConfigFile(
  filePath: string | undefined,
  cwd: string,
): ConfigResult<PartialClawConfig> {
  if (!filePath) return { ok: true, value: {} }

  const absPath = isAbsolute(filePath) ? filePath : resolve(cwd, filePath)
  if (!existsSync(absPath)) {
    return { ok: false, error: `Config file not found: ${absPath}` }
  }

  let raw: unknown
  try {
    const doc = parseDocument(readFileSync(absPath, 'utf-8'))
    if (doc.errors.length > 0) {
      return {
        ok: false,
        error: `Config file error: ${doc.errors.map((err) => err.message).join('\n')}`,
      }
    }
    raw = doc.toJS()
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    return { ok: false, error: `Config file error: ${message}` }
  }

  const parsed = ClawConfigFileSchema.safeParse(raw ?? {})
  if (!parsed.success) {
    return {
      ok: false,
      error: `Config file error:\n${formatZodIssues(parsed.error.issues)}`,
    }
  }

  return {
    ok: true,
    value: omitUndefined({
      port: parsed.data.ports?.server,
      cdpPort: parsed.data.ports?.cdp,
    }),
  }
}

function normalizePortInput(value: unknown): unknown {
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'string') return value

  const trimmed = value.trim()
  return trimmed === '' ? undefined : Number(trimmed)
}

function toUserArgs(argv: string[]): string[] {
  return argv.length >= 2 && !argv[0]?.startsWith('-') ? argv.slice(2) : argv
}

function formatZodIssues(
  issues: ConfigIssue[],
  pathLabels: Record<string, string> = {},
): string {
  return issues
    .map((issue) => {
      const path = issue.path.map(String).join('.')
      const source = pathLabels[path] ?? (path || 'config')
      let message = issue.message

      if (issue.code === 'unrecognized_keys' && issue.keys) {
        message = `unknown key(s): ${issue.keys.join(', ')}`
      } else if (isPortSource(source)) {
        message = 'must be an integer port between 1 and 65535'
      }

      return `  - ${source}: ${message}`
    })
    .join('\n')
}

function isPortSource(source: string): boolean {
  return [
    'port',
    'cdpPort',
    'ports.server',
    'ports.cdp',
    'CLAW_SERVER_PORT',
    'BROWSEROS_CLAW_CDP_PORT',
  ].includes(source)
}

function getDefaults(): ClawConfig {
  return {
    port: CLAW_API_PORT_DEFAULT,
    cdpPort: CLAW_CDP_PORT_DEFAULT,
  }
}

function mergeConfigs(...configs: PartialClawConfig[]): PartialClawConfig {
  const result: PartialClawConfig = {}
  for (const config of configs) {
    for (const [key, value] of Object.entries(config)) {
      if (value !== undefined) {
        ;(result as Record<string, unknown>)[key] = value
      }
    }
  }
  return result
}

function omitUndefined<T extends Record<string, unknown>>(obj: T): Partial<T> {
  return Object.fromEntries(
    Object.entries(obj).filter(([, value]) => value !== undefined),
  ) as Partial<T>
}

function cleanString(value: string | undefined): string | undefined {
  const trimmed = value?.trim()
  return trimmed ? trimmed : undefined
}
