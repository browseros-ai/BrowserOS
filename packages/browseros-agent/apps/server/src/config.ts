/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Server configuration loading with multiple sources.
 * Precedence: CLI > Config File > Environment > Defaults
 */
import fs from 'node:fs'
import path from 'node:path'

import { Command, InvalidArgumentError } from 'commander'
import { z } from 'zod'

import { INLINED_ENV, REQUIRED_FOR_PRODUCTION } from './env'
import { VERSION } from './version'

const portSchema = z.number().int()

export const ServerConfigSchema = z.object({
  cdpPort: portSchema.nullable(),
  serverPort: portSchema,
  agentPort: portSchema,
  extensionPort: portSchema.nullable(),
  resourcesDir: z.string(),
  executionDir: z.string(),
  mcpAllowRemote: z.boolean(),
  codegenServiceUrl: z.string().optional(),
  instanceClientId: z.string().optional(),
  instanceInstallId: z.string().optional(),
  instanceBrowserosVersion: z.string().optional(),
  instanceChromiumVersion: z.string().optional(),
  aiSdkDevtoolsEnabled: z.boolean(),
})

export type ServerConfig = z.infer<typeof ServerConfigSchema>

type PartialConfig = Partial<z.input<typeof ServerConfigSchema>>

export type ConfigResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: string }

interface ParsedCliArgs {
  configPath?: string
  cwd: string
  overrides: PartialConfig
}

export function loadServerConfig(
  argv: string[] = process.argv,
): ConfigResult<ServerConfig> {
  // 1. Parse CLI args
  const cli = parseCliArgs(argv)
  if (!cli.ok) return cli

  // 2. Parse config file (only if --config provided)
  const file = parseConfigFile(cli.value.configPath)
  if (!file.ok) return file

  // 3. Parse runtime environment variables
  const runtimeEnv = parseRuntimeEnv()

  // 4. Merge: Defaults < Env < File < CLI
  const merged = mergeConfigs(
    getDefaults(cli.value.cwd),
    runtimeEnv,
    file.value,
    cli.value.overrides,
  )

  // 5. Add build-time inlined values
  merged.codegenServiceUrl = INLINED_ENV.CODEGEN_SERVICE_URL

  // 6. agentPort is deprecated - always equals serverPort
  merged.agentPort = merged.serverPort

  // 7. Validate with Zod
  const result = ServerConfigSchema.safeParse(merged)
  if (!result.success) {
    const errors = result.error.issues
      .map((i) => `  - ${i.path.join('.')}: ${i.message}`)
      .join('\n')
    return {
      ok: false,
      error: `Invalid server configuration:\n${errors}\n\nProvide via --config, CLI flags, or environment variables.`,
    }
  }

  // 8. Validate required inlined env vars for production
  const inlinedValidation = validateInlinedEnv()
  if (!inlinedValidation.ok) return inlinedValidation

  return { ok: true, value: result.data }
}

function parseCliArgs(argv: string[]): ConfigResult<ParsedCliArgs> {
  const program = new Command()

  try {
    program
      .name('browseros-server')
      .description('BrowserOS Unified Server - MCP + Agent')
      .version(VERSION)
      .option('--config <path>', 'Path to JSON configuration file')
      .option(
        '--cdp-port <port>',
        'CDP WebSocket port (optional)',
        parsePortArg,
      )
      .option('--server-port <port>', 'Server HTTP port', parsePortArg)
      .option(
        '--http-mcp-port <port>',
        '[DEPRECATED] Use --server-port',
        parsePortArg,
      )
      .option(
        '--agent-port <port>',
        '[DEPRECATED] Use --server-port',
        parsePortArg,
      )
      .option(
        '--extension-port <port>',
        '[DEPRECATED] No-op, kept for backwards compatibility',
        parsePortArg,
      )
      .option('--resources-dir <path>', 'Resources directory path')
      .option(
        '--execution-dir <path>',
        'Execution directory for logs and configs',
      )
      .option(
        '--allow-remote-in-mcp',
        'Allow non-localhost MCP connections',
        false,
      )
      .option(
        '--disable-mcp-server',
        '[DEPRECATED] No-op, kept for backwards compatibility',
      )
      .exitOverride((err) => {
        if (err.exitCode === 0) {
          process.exit(0)
        }
        throw err
      })
      .parse(argv)
  } catch (e: unknown) {
    const message = e instanceof Error ? e.message : String(e)
    return { ok: false, error: message }
  }

  const opts = program.opts()

  if (opts.httpMcpPort !== undefined) {
    console.warn('Warning: --http-mcp-port is deprecated. Use --server-port.')
  }

  if (opts.agentPort !== undefined) {
    console.warn(
      'Warning: --agent-port is deprecated and has no effect. Use --server-port.',
    )
  }

  if (opts.extensionPort !== undefined) {
    console.warn('Warning: --extension-port is deprecated and has no effect.')
  }

  const cwd = process.cwd()

  return {
    ok: true,
    value: {
      configPath: opts.config,
      cwd,
      overrides: omitUndefined({
        cdpPort: opts.cdpPort,
        serverPort: opts.serverPort ?? opts.httpMcpPort,
        extensionPort: opts.extensionPort,
        resourcesDir: opts.resourcesDir
          ? toAbsolutePath(opts.resourcesDir, cwd)
          : undefined,
        executionDir: opts.executionDir
          ? toAbsolutePath(opts.executionDir, cwd)
          : undefined,
        mcpAllowRemote: opts.allowRemoteInMcp || undefined,
      }),
    },
  }
}

function parsePortArg(value: string): number {
  const port = parseInt(value, 10)
  if (Number.isNaN(port)) {
    throw new InvalidArgumentError('Not a valid port number')
  }
  return port
}

function parseConfigFile(filePath?: string): ConfigResult<PartialConfig> {
  if (!filePath) {
    return { ok: true, value: {} }
  }

  const absPath = path.isAbsolute(filePath)
    ? filePath
    : path.resolve(process.cwd(), filePath)

  if (!fs.existsSync(absPath)) {
    return { ok: false, error: `Config file not found: ${absPath}` }
  }

  try {
    const content = fs.readFileSync(absPath, 'utf-8')
    const cfg = JSON.parse(content)
    const configDir = path.dirname(absPath)

    return {
      ok: true,
      value: omitUndefined({
        cdpPort: cfg.ports?.cdp,
        serverPort: cfg.ports?.server ?? cfg.ports?.http_mcp,
        extensionPort: cfg.ports?.extension,
        resourcesDir: parseAbsolutePath(cfg.directories?.resources, configDir),
        executionDir: parseAbsolutePath(cfg.directories?.execution, configDir),
        mcpAllowRemote:
          cfg.flags?.allow_remote_in_mcp === true ? true : undefined,
        aiSdkDevtoolsEnabled:
          cfg.flags?.ai_sdk_devtools === true ? true : undefined,
        instanceClientId:
          typeof cfg.instance?.client_id === 'string'
            ? cfg.instance.client_id
            : undefined,
        instanceInstallId:
          typeof cfg.instance?.install_id === 'string'
            ? cfg.instance.install_id
            : undefined,
        instanceBrowserosVersion:
          typeof cfg.instance?.browseros_version === 'string'
            ? cfg.instance.browseros_version
            : undefined,
        instanceChromiumVersion:
          typeof cfg.instance?.chromium_version === 'string'
            ? cfg.instance.chromium_version
            : undefined,
      }),
    }
  } catch (e: unknown) {
    const message = e instanceof Error ? e.message : String(e)
    return { ok: false, error: `Config file error: ${message}` }
  }
}

function parseRuntimeEnv(): PartialConfig {
  const cwd = process.cwd()
  return omitUndefined({
    cdpPort: process.env.BROWSEROS_CDP_PORT
      ? safeParseInt(process.env.BROWSEROS_CDP_PORT)
      : undefined,
    serverPort: process.env.BROWSEROS_SERVER_PORT
      ? safeParseInt(process.env.BROWSEROS_SERVER_PORT)
      : undefined,
    extensionPort: process.env.BROWSEROS_EXTENSION_PORT
      ? safeParseInt(process.env.BROWSEROS_EXTENSION_PORT)
      : undefined,
    resourcesDir: process.env.BROWSEROS_RESOURCES_DIR
      ? toAbsolutePath(process.env.BROWSEROS_RESOURCES_DIR, cwd)
      : undefined,
    executionDir: process.env.BROWSEROS_EXECUTION_DIR
      ? toAbsolutePath(process.env.BROWSEROS_EXECUTION_DIR, cwd)
      : undefined,
    instanceInstallId: process.env.BROWSEROS_INSTALL_ID,
    instanceClientId: process.env.BROWSEROS_CLIENT_ID,
    aiSdkDevtoolsEnabled:
      process.env.BROWSEROS_AI_SDK_DEVTOOLS === 'true' ? true : undefined,
  })
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

function getDefaults(cwd: string): PartialConfig {
  return {
    cdpPort: null,
    extensionPort: null,
    resourcesDir: cwd,
    executionDir: cwd,
    mcpAllowRemote: false,
    aiSdkDevtoolsEnabled: false,
  }
}

function mergeConfigs(...configs: PartialConfig[]): PartialConfig {
  const result: PartialConfig = {}
  for (const config of configs) {
    for (const [key, value] of Object.entries(config)) {
      if (value !== undefined) {
        ;(result as Record<string, unknown>)[key] = value
      }
    }
  }
  return result
}

function safeParseInt(value: string): number | undefined {
  const num = parseInt(value, 10)
  return Number.isNaN(num) ? undefined : num
}

function omitUndefined<T extends Record<string, unknown>>(obj: T): Partial<T> {
  return Object.fromEntries(
    Object.entries(obj).filter(([_, v]) => v !== undefined),
  ) as Partial<T>
}

function toAbsolutePath(target: string, baseDir: string): string {
  return path.isAbsolute(target) ? target : path.resolve(baseDir, target)
}

function parseAbsolutePath(val: unknown, baseDir: string): string | undefined {
  if (typeof val !== 'string') return undefined
  return toAbsolutePath(val, baseDir)
}
