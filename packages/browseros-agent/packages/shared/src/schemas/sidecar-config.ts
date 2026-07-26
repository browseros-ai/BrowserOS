import { existsSync, readFileSync } from 'node:fs'
import { dirname, isAbsolute, resolve } from 'node:path'
import { z } from 'zod'

export type SidecarConfigResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: string }

export interface LoadSidecarConfigOptions {
  cwd?: string
}

export interface SidecarConfig {
  ports: {
    server?: number
    cdp?: number
    proxy?: number
  }
  directories: {
    resources?: string
    execution?: string
  }
  flags: {
    allow_remote_in_mcp?: boolean
  }
  instance: {
    client_id?: string
    install_id?: string
    browseros_version?: string
    chromium_version?: string
  }
  replay?: {
    retentionDays?: number
  }
}

const portSchema = z.preprocess(
  normalizePortInput,
  z
    .number({
      invalid_type_error: 'must be an integer port between 1 and 65535',
    })
    .int('must be an integer port between 1 and 65535')
    .min(1, 'must be an integer port between 1 and 65535')
    .max(65535, 'must be an integer port between 1 and 65535'),
)

const pathSchema = z.preprocess(
  normalizePathInput,
  z.string().min(1, 'must be a non-empty path'),
)

const SidecarConfigFileSchema = z
  .object({
    ports: z
      .object({
        server: portSchema.optional(),
        cdp: portSchema.optional(),
        proxy: portSchema.optional(),
      })
      .passthrough()
      .optional(),
    directories: z
      .object({
        resources: pathSchema.optional(),
        execution: pathSchema.optional(),
      })
      .passthrough()
      .optional(),
    flags: z
      .object({
        allow_remote_in_mcp: z.boolean().optional(),
      })
      .passthrough()
      .optional(),
    instance: z
      .object({
        client_id: z.string().optional(),
        install_id: z.string().optional(),
        browseros_version: z.string().optional(),
        chromium_version: z.string().optional(),
      })
      .passthrough()
      .optional(),
    replay: z
      .object({
        retentionDays: z.number().int().positive().optional(),
      })
      .passthrough()
      .optional(),
  })
  .passthrough()

type ParsedSidecarConfigFile = z.infer<typeof SidecarConfigFileSchema>

/** Reads and validates a Chromium-authored sidecar JSON config file. */
export function parseSidecarConfigFile(
  filePath: string,
  options: LoadSidecarConfigOptions = {},
): SidecarConfigResult<SidecarConfig> {
  const absPath = resolveConfigPath(filePath, options.cwd ?? process.cwd())
  if (!existsSync(absPath)) {
    return { ok: false, error: `Sidecar config file not found: ${absPath}` }
  }

  let raw: unknown
  try {
    raw = JSON.parse(readFileSync(absPath, 'utf-8'))
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    return { ok: false, error: `Sidecar config file error: ${message}` }
  }

  const parsed = SidecarConfigFileSchema.safeParse(raw)
  if (!parsed.success) {
    return {
      ok: false,
      error: `Invalid sidecar configuration:\n${formatZodIssues(
        parsed.error.issues,
      )}`,
    }
  }

  return {
    ok: true,
    value: projectSidecarConfig(parsed.data, dirname(absPath)),
  }
}

function projectSidecarConfig(
  parsed: ParsedSidecarConfigFile,
  configDir: string,
): SidecarConfig {
  const replay = omitUndefined({
    retentionDays: parsed.replay?.retentionDays,
  })
  return {
    ports: omitUndefined({
      server: parsed.ports?.server,
      cdp: parsed.ports?.cdp,
      proxy: parsed.ports?.proxy,
    }),
    directories: omitUndefined({
      resources: resolvePath(parsed.directories?.resources, configDir),
      execution: resolvePath(parsed.directories?.execution, configDir),
    }),
    flags: omitUndefined({
      allow_remote_in_mcp: parsed.flags?.allow_remote_in_mcp,
    }),
    instance: omitUndefined({
      client_id: parsed.instance?.client_id,
      install_id: parsed.instance?.install_id,
      browseros_version: parsed.instance?.browseros_version,
      chromium_version: parsed.instance?.chromium_version,
    }),
    ...(Object.keys(replay).length > 0 && { replay }),
  }
}

function normalizePortInput(value: unknown): unknown {
  if (typeof value !== 'string') return value
  const trimmed = value.trim()
  if (!/^\d+$/.test(trimmed)) return value
  return Number(trimmed)
}

function normalizePathInput(value: unknown): unknown {
  if (typeof value !== 'string') return value
  return value.trim()
}

function resolveConfigPath(filePath: string, cwd: string): string {
  return isAbsolute(filePath) ? filePath : resolve(cwd, filePath)
}

function resolvePath(
  value: string | undefined,
  baseDir: string,
): string | undefined {
  if (value === undefined) return undefined
  return isAbsolute(value) ? value : resolve(baseDir, value)
}

function formatZodIssues(issues: z.ZodIssue[]): string {
  return issues
    .map((issue) => {
      const path = issue.path.map(String).join('.') || 'config'
      return `  - ${path}: ${issue.message}`
    })
    .join('\n')
}

function omitUndefined<T extends Record<string, unknown>>(obj: T): Partial<T> {
  return Object.fromEntries(
    Object.entries(obj).filter(([, value]) => value !== undefined),
  ) as Partial<T>
}
