/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { spawnSync } from 'node:child_process'

export interface ResolveLoginShellPathOptions {
  platform?: NodeJS.Platform
  env?: NodeJS.ProcessEnv
  run?: typeof spawnSync
}

// undefined = not yet resolved; null = resolved but unavailable; string = PATH.
let cache: string | null | undefined

/**
 * The PATH from the user's login shell.
 *
 * A GUI-launched server (double-clicked app, esp. on macOS) inherits a minimal
 * PATH that omits shell-profile additions (homebrew, nvm/fnm, ~/.local/bin), so
 * a custom agent's binary may be unfindable even though it works in a terminal.
 * Ask the login shell for its PATH once and cache it. Windows inherits the full
 * registry PATH already, so there is nothing to resolve there.
 *
 * Returns undefined when unavailable (Windows, resolution error, or timeout);
 * callers fall back to the inherited PATH.
 */
export function resolveLoginShellPath(
  options: ResolveLoginShellPathOptions = {},
): string | undefined {
  const platform = options.platform ?? process.platform
  if (platform === 'win32') return undefined
  if (cache !== undefined) return cache ?? undefined
  cache = computeLoginShellPath(options) ?? null
  return cache ?? undefined
}

/** Reset the module cache. Test-only. */
export function resetLoginShellPathCache(): void {
  cache = undefined
}

function computeLoginShellPath(
  options: ResolveLoginShellPathOptions,
): string | undefined {
  const shell = options.env?.SHELL ?? process.env.SHELL ?? '/bin/bash'
  const run = options.run ?? spawnSync
  try {
    const result = run(shell, ['-lc', 'printf %s "$PATH"'], {
      encoding: 'utf8',
      timeout: 5_000,
      stdio: ['ignore', 'pipe', 'ignore'],
    })
    const path = typeof result.stdout === 'string' ? result.stdout.trim() : ''
    return path.length > 0 ? path : undefined
  } catch {
    return undefined
  }
}

/** Prepend the login-shell PATH to the inherited PATH, de-duplicated. */
export function mergePath(loginPath: string, inheritedPath: string): string {
  const seen = new Set<string>()
  const merged: string[] = []
  for (const entry of `${loginPath}:${inheritedPath}`.split(':')) {
    if (!entry || seen.has(entry)) continue
    seen.add(entry)
    merged.push(entry)
  }
  return merged.join(':')
}
