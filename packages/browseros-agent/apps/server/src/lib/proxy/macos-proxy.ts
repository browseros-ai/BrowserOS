/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * macOS system proxy discovery via `scutil --proxy`.
 *
 * Fallback for GUI-launched servers whose environment lacks HTTP_PROXY even
 * though a proxy is set in System Settings. Only runs on darwin; elsewhere
 * returns undefined without spawning anything.
 */

import { spawnSync } from 'node:child_process'

export interface MacOSProxySettings {
  httpProxy: string | undefined
  httpsProxy: string | undefined
  exceptions: string[]
  excludeSimpleHostnames: boolean
}

export interface MacOSProxyReaderOptions {
  platform?: NodeJS.Platform
  run?: typeof spawnSync
}

let cache: MacOSProxySettings | null | undefined

/** Reset the module cache. Test-only. */
export function resetMacOSProxyCache(): void {
  cache = undefined
}

export function readMacOSProxySettings(
  options: MacOSProxyReaderOptions = {},
): MacOSProxySettings | undefined {
  const platform = options.platform ?? process.platform
  if (platform !== 'darwin') return undefined
  if (cache !== undefined) return cache ?? undefined
  cache = computeMacOSProxySettings(options) ?? null
  return cache ?? undefined
}

function computeMacOSProxySettings(
  options: MacOSProxyReaderOptions,
): MacOSProxySettings | undefined {
  const run = options.run ?? spawnSync
  try {
    const result = run('scutil', ['--proxy'], {
      encoding: 'utf8',
      timeout: 5_000,
      stdio: ['ignore', 'pipe', 'ignore'],
    })
    const stdout = typeof result.stdout === 'string' ? result.stdout : ''
    if (!stdout) return undefined
    return parseScutilOutput(stdout)
  } catch {
    return undefined
  }
}

const KEY_VALUE = /^\s*([A-Za-z]+)\s*:\s*(.*?)\s*$/
const EXCEPTION_ENTRY = /^\s*\d+\s*:\s*(.*?)\s*$/

/** Parse `scutil --proxy` output. Exported for tests. */
export function parseScutilOutput(
  stdout: string,
): MacOSProxySettings | undefined {
  const values = new Map<string, string>()
  const exceptions: string[] = []
  let inExceptions = false
  for (const line of stdout.split('\n')) {
    const exception = inExceptions ? EXCEPTION_ENTRY.exec(line) : null
    if (exception) {
      if (exception[1]) exceptions.push(exception[1])
      continue
    }
    const match = KEY_VALUE.exec(line)
    if (!match) {
      inExceptions = false
      continue
    }
    const [, key, value] = match
    if (key === 'ExceptionsList') {
      inExceptions = true
      continue
    }
    inExceptions = false
    values.set(key, value)
  }
  if (values.size === 0 && exceptions.length === 0) return undefined
  const httpEnabled = values.get('HTTPEnable') === '1'
  const httpsEnabled = values.get('HTTPSEnable') === '1'
  const httpHost = values.get('HTTPProxy')
  const httpsHost = values.get('HTTPSProxy')
  return {
    httpProxy:
      httpEnabled && httpHost
        ? `http://${httpHost}:${values.get('HTTPPort') ?? '8080'}`
        : undefined,
    httpsProxy:
      httpsEnabled && httpsHost
        ? `http://${httpsHost}:${values.get('HTTPSPort') ?? '8080'}`
        : undefined,
    exceptions,
    excludeSimpleHostnames: values.get('ExcludeSimpleHostnames') === '1',
  }
}
