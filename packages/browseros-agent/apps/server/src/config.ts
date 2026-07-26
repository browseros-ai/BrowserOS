import {
  parseSidecarConfigFile,
  type SidecarConfig,
} from '@browseros/shared/schemas/sidecar-config'
import { Command } from 'commander'
import { z } from 'zod'
import { INLINED_ENV, REQUIRED_FOR_PRODUCTION } from './env'
import { VERSION } from './version'

const portSchema = z.number().int().min(1).max(65535)

const ServerConfigSchema = z.object({
  cdpPort: portSchema,
  serverPort: portSchema,
  agentPort: portSchema,
  extensionPort: portSchema.nullable(),
  resourcesDir: z.string().min(1),
  executionDir: z.string().min(1),
  mcpAllowRemote: z.boolean(),
  instanceClientId: z.string().optional(),
  instanceInstallId: z.string().optional(),
  instanceBrowserosVersion: z.string().optional(),
  instanceChromiumVersion: z.string().optional(),
  aiSdkDevtoolsEnabled: z.boolean(),
})

export type ServerConfig = z.infer<typeof ServerConfigSchema>

export type ConfigResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: string }

interface ParsedCliArgs {
  configPath: string
}

/** Loads BrowserOS server config from the sidecar JSON file passed by Chromium. */
export function loadServerConfig(
  argv: string[] = process.argv,
): ConfigResult<ServerConfig> {
  const cli = parseCliArgs(argv)
  if (!cli.ok) return cli

  const sidecar = parseSidecarConfigFile(cli.value.configPath)
  if (!sidecar.ok) return sidecar

  const projected = projectServerConfig(sidecar.value)
  if (!projected.ok) return projected

  const inlinedValidation = validateInlinedEnv()
  if (!inlinedValidation.ok) return inlinedValidation

  return projected
}

function parseCliArgs(argv: string[]): ConfigResult<ParsedCliArgs> {
  const program = new Command()

  try {
    program
      .name('browseros-server')
      .description('BrowserOS Unified Server - MCP + Agent')
      .version(VERSION)
      .option('--config <path>', 'Path to sidecar JSON configuration file')
      .configureOutput({ writeErr: () => {} })
      .exitOverride((err) => {
        if (err.exitCode === 0) process.exit(0)
        throw err
      })
      .parse(toUserArgs(argv), { from: 'user' })
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    return { ok: false, error: message }
  }

  const opts = program.opts<{ config?: string }>()
  const configPath = cleanString(opts.config)
  if (program.getOptionValueSource('config') === 'cli' && !configPath) {
    return { ok: false, error: '--config requires a path' }
  }
  if (!configPath) {
    return { ok: false, error: '--config is required' }
  }

  return { ok: true, value: { configPath } }
}

function projectServerConfig(
  sidecar: SidecarConfig,
): ConfigResult<ServerConfig> {
  const missing = requiredSidecarFields(sidecar)
  if (missing.length > 0) {
    return {
      ok: false,
      error: `Invalid server sidecar configuration:\n${missing
        .map((field) => `  - ${field}: Required`)
        .join('\n')}`,
    }
  }

  const result = ServerConfigSchema.safeParse({
    cdpPort: sidecar.ports.cdp,
    serverPort: sidecar.ports.server,
    agentPort: sidecar.ports.server,
    extensionPort: null,
    resourcesDir: sidecar.directories.resources,
    executionDir: sidecar.directories.execution,
    mcpAllowRemote: sidecar.flags.allow_remote_in_mcp ?? false,
    instanceClientId: sidecar.instance.client_id,
    instanceInstallId: sidecar.instance.install_id,
    instanceBrowserosVersion: sidecar.instance.browseros_version,
    instanceChromiumVersion: sidecar.instance.chromium_version,
    aiSdkDevtoolsEnabled: process.env.BROWSEROS_AI_SDK_DEVTOOLS === 'true',
  })
  if (!result.success) {
    return {
      ok: false,
      error: `Invalid server sidecar configuration:\n${formatZodIssues(
        result.error.issues,
      )}`,
    }
  }

  return { ok: true, value: result.data }
}

function requiredSidecarFields(sidecar: SidecarConfig): string[] {
  const fields: Array<[string, unknown]> = [
    ['ports.server', sidecar.ports.server],
    ['ports.cdp', sidecar.ports.cdp],
    ['directories.resources', sidecar.directories.resources],
    ['directories.execution', sidecar.directories.execution],
  ]
  return fields
    .filter(([, value]) => value === undefined)
    .map(([field]) => field)
}

function validateInlinedEnv(): ConfigResult<void> {
  if (process.env.NODE_ENV !== 'production') {
    return { ok: true, value: undefined }
  }

  const missing: string[] = []
  for (const varName of REQUIRED_FOR_PRODUCTION) {
    if (!INLINED_ENV[varName]) {
      missing.push(varName)
    }
  }

  if (missing.length > 0) {
    return {
      ok: false,
      error: `Missing required environment variables for production:\n${missing.map((v) => `  - ${v}`).join('\n')}`,
    }
  }

  return { ok: true, value: undefined }
}

function toUserArgs(argv: string[]): string[] {
  if (argv.length === 0 || argv[0]?.startsWith('-')) return argv
  return argv[1]?.startsWith('-') ? argv.slice(1) : argv.slice(2)
}

function cleanString(value: string | undefined): string | undefined {
  const trimmed = value?.trim()
  return trimmed ? trimmed : undefined
}

function formatZodIssues(issues: z.ZodIssue[]): string {
  return issues
    .map((issue) => {
      const path = issue.path.map(String).join('.') || 'config'
      return `  - ${path}: ${issue.message}`
    })
    .join('\n')
}
