/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Windows system proxy discovery via the registry.
 *
 * Reads HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings with
 * `reg query` (ships with Windows, no native modules) instead of
 * registry-js, so it works in the compiled Bun sidecar. Only runs on win32;
 * elsewhere returns undefined without spawning anything.
 */

import { spawnSync } from 'node:child_process'

const REGISTRY_KEY =
  'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings'

export interface WindowsProxySettings {
  enabled: boolean
  proxyServer: string | undefined
  proxyOverride: string | undefined
  autoConfigUrl: string | undefined
}

export interface WindowsProxyReaderOptions {
  platform?: NodeJS.Platform
  run?: typeof spawnSync
}

let cache: WindowsProxySettings | null | undefined

/** Reset the module cache. Test-only. */
export function resetWindowsProxyCache(): void {
  cache = undefined
}

export function readWindowsProxySettings(
  options: WindowsProxyReaderOptions = {},
): WindowsProxySettings | undefined {
  const platform = options.platform ?? process.platform
  if (platform !== 'win32') return undefined
  if (cache !== undefined) return cache ?? undefined
  cache = computeWindowsProxySettings(options) ?? null
  return cache ?? undefined
}

function computeWindowsProxySettings(
  options: WindowsProxyReaderOptions,
): WindowsProxySettings | undefined {
  const run = options.run ?? spawnSync
  try {
    const result = run('reg', ['query', REGISTRY_KEY], {
      encoding: 'utf8',
      timeout: 5_000,
      stdio: ['ignore', 'pipe', 'ignore'],
    })
    const stdout = typeof result.stdout === 'string' ? result.stdout : ''
    if (!stdout) return undefined
    return parseRegQueryOutput(stdout)
  } catch {
    return undefined
  }
}

const VALUE_LINE = /^\s+(\S+)\s+REG_\S+\s+(.*\S)\s*$/

/** Parse `reg query` output into proxy settings. Exported for tests. */
export function parseRegQueryOutput(
  stdout: string,
): WindowsProxySettings | undefined {
  const values = new Map<string, string>()
  for (const line of stdout.split('\n')) {
    const match = VALUE_LINE.exec(line)
    if (match) values.set(match[1].toLowerCase(), match[2].trim())
  }
  if (values.size === 0) return undefined
  return {
    enabled: values.get('proxyenable') !== '0x0',
    proxyServer: values.get('proxyserver'),
    proxyOverride: values.get('proxyoverride'),
    autoConfigUrl: values.get('autoconfigurl'),
  }
}

export interface ManualProxy {
  http: string | undefined
  https: string | undefined
}

/**
 * Parse a ProxyServer value: `http=host:port;https=host:port` per-scheme
 * entries, or a bare `host:port` applying to all schemes. SOCKS entries are
 * dropped — Bun's fetch proxy is HTTP(S)-only.
 */
export function parseProxyServer(value: string | undefined): ManualProxy {
  const manual: ManualProxy = { http: undefined, https: undefined }
  if (!value) return manual
  for (const entry of value.split(';')) {
    const trimmed = entry.trim()
    if (!trimmed) continue
    const equals = trimmed.indexOf('=')
    if (equals === -1) {
      const url = withScheme(trimmed)
      manual.http ??= url
      manual.https ??= url
      continue
    }
    const scheme = trimmed.slice(0, equals).trim().toLowerCase()
    const url = withScheme(trimmed.slice(equals + 1).trim())
    if (!url) continue
    if (scheme === 'http') manual.http ??= url
    else if (scheme === 'https') manual.https ??= url
  }
  return manual
}

function withScheme(value: string): string | undefined {
  const trimmed = value.trim()
  if (!trimmed) return undefined
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(trimmed)) return trimmed
  return `http://${trimmed}`
}

/** Split a ProxyOverride value (`;`-separated) into bypass patterns. */
export function parseProxyOverride(value: string | undefined): string[] {
  if (!value) return []
  return value
    .split(';')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
}
