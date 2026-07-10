import * as fsp from 'node:fs/promises'

/**
 * Resolves `$VAR` segments against `process.env`. Mirrors Docker's
 * `isPathValid` / `os.ExpandEnv` pair: returns null if any referenced
 * env var is undefined or empty.
 */
const ENV_VAR_RE = /\$([A-Za-z_][A-Za-z0-9_]*)/g

function expandPath(p: string): string | null {
  let missing = false
  const out = p.replace(ENV_VAR_RE, (_, name: string) => {
    const value = process.env[name]
    if (!value) {
      missing = true
      return ''
    }
    return value
  })
  return missing ? null : out
}

export function expandPaths(paths: string[]): string[] {
  const out: string[] = []
  for (const raw of paths) {
    const expanded = expandPath(raw)
    if (expanded !== null) out.push(expanded)
  }
  return out
}

export async function pathExists(p: string): Promise<boolean> {
  try {
    await fsp.stat(p)
    return true
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return false
    return false
  }
}

export async function anyExists(paths: string[]): Promise<boolean> {
  for (const p of paths) {
    if (await pathExists(p)) return true
  }
  return false
}

/**
 * Pick the best system config path: prefer the first one that already
 * exists; otherwise the first one with all env vars resolvable.
 */
export async function pickConfigPath(
  candidates: string[],
): Promise<string | null> {
  const expanded = expandPaths(candidates)
  for (const p of expanded) {
    if (await pathExists(p)) return p
  }
  return expanded[0] ?? null
}
