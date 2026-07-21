import { dirname, resolve } from 'node:path'
import {
  parseSidecarConfigFile,
  type SidecarConfig,
} from '@browseros/shared/schemas/sidecar-config'
import { Command } from 'commander'
import { z } from 'zod'
import { VERSION } from './version'

const portSchema = z.number().int().min(1).max(65535)

const ClawConfigSchema = z.object({
  serverPort: portSchema,
  cdpPort: portSchema,
  proxyPort: portSchema.optional(),
  resourcesDir: z.string().min(1),
  replay: z.object({ retentionDays: z.number().int().positive() }).optional(),
})

export type ClawConfig = z.infer<typeof ClawConfigSchema>
export type ConfigResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: string }

export interface LoadClawConfigOptions {
  argv?: string[]
  cwd?: string
  env?: Record<string, string | undefined>
}

export interface DefaultResourcesRuntime {
  execPath?: string
  importMetaPath?: string
  isStandalone?: boolean
}

type ParsedCliArgs = {
  configPath: string
}

/** Loads Claw startup config from the Chromium sidecar JSON file. */
export function loadClawConfig(
  options: LoadClawConfigOptions = {},
): ConfigResult<ClawConfig> {
  const argv = options.argv ?? process.argv
  const cwd = options.cwd ?? process.cwd()

  const cli = parseCliArgs(argv)
  if (!cli.ok) return cli

  const sidecar = parseSidecarConfigFile(cli.value.configPath, { cwd })
  if (!sidecar.ok) return sidecar

  return projectClawConfig(sidecar.value, cwd)
}

function parseCliArgs(argv: string[]): ConfigResult<ParsedCliArgs> {
  const program = new Command()

  try {
    program
      .name('claw-server')
      .description('BrowserClaw standalone API')
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

function projectClawConfig(
  sidecar: SidecarConfig,
  cwd: string,
): ConfigResult<ClawConfig> {
  const missing = requiredSidecarFields(sidecar)
  if (missing.length > 0) {
    return {
      ok: false,
      error: `Invalid Claw sidecar configuration:\n${missing
        .map((field) => `  - ${field}: Required`)
        .join('\n')}`,
    }
  }

  const result = ClawConfigSchema.safeParse({
    serverPort: sidecar.ports.server,
    cdpPort: sidecar.ports.cdp,
    proxyPort: sidecar.ports.proxy,
    resourcesDir:
      sidecar.directories.resources ?? resolveDefaultResourcesDir(cwd),
    replay: sidecar.replay,
  })
  if (!result.success) {
    return {
      ok: false,
      error: `Invalid Claw sidecar configuration:\n${formatZodIssues(
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
  ]
  return fields
    .filter(([, value]) => value === undefined)
    .map(([field]) => field)
}

/** Resolves Claw's resource root for source runs and packaged binaries. */
export function resolveDefaultResourcesDir(
  cwd = process.cwd(),
  runtime: DefaultResourcesRuntime = {},
): string {
  const standalone =
    runtime.isStandalone ?? isStandaloneExecutable(runtime.importMetaPath)
  if (standalone) {
    return resolve(dirname(runtime.execPath ?? process.execPath), '..')
  }
  return resolve(cwd, 'resources')
}

function isStandaloneExecutable(importMetaPath = import.meta.path): boolean {
  const bunWithStandaloneFlag = Bun as { isStandaloneExecutable?: boolean }
  return (
    bunWithStandaloneFlag.isStandaloneExecutable === true ||
    importMetaPath.startsWith('/$bunfs/')
  )
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
