/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * System proxy resolution for outbound LLM traffic.
 *
 * Precedence per target URL: explicit env proxy (HTTP_PROXY/HTTPS_PROXY)
 * first, then the OS system setting (Windows registry incl. PAC, macOS
 * scutil), else direct. Bypass lists (NO_PROXY, ProxyOverride, scutil
 * exceptions, plus localhost defaults) always win. Returns a proxy URL
 * suitable for Bun's fetch `proxy` option, or undefined for DIRECT.
 */

import type { spawnSync } from 'node:child_process'
import { DEFAULT_BYPASS, isBypassed } from './bypass'
import { readMacOSProxySettings } from './macos-proxy'
import { evaluatePacUrl, type PacVerdict } from './pac-evaluator'
import { readProxyEnv } from './proxy-env'
import {
  parseProxyOverride,
  parseProxyServer,
  readWindowsProxySettings,
} from './windows-registry'

export interface ResolveProxyOptions {
  platform?: NodeJS.Platform
  env?: NodeJS.ProcessEnv
  run?: typeof spawnSync
  evaluatePac?: (pacUrl: string, targetUrl: string) => Promise<PacVerdict>
}

function targetParts(
  targetUrl: string,
): { hostname: string; port: string; isHttps: boolean } | undefined {
  try {
    const parsed = new URL(targetUrl)
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return undefined
    }
    return {
      hostname: parsed.hostname,
      port: parsed.port || (parsed.protocol === 'https:' ? '443' : '80'),
      isHttps: parsed.protocol === 'https:',
    }
  } catch {
    return undefined
  }
}

export async function resolveProxyForUrl(
  targetUrl: string,
  options: ResolveProxyOptions = {},
): Promise<string | undefined> {
  const target = targetParts(targetUrl)
  if (!target) return undefined
  const platform = options.platform ?? process.platform
  const env = options.env ?? process.env
  const { httpProxy, httpsProxy, noProxy } = readProxyEnv(env)
  const patterns = [...DEFAULT_BYPASS, ...noProxy]

  // Env proxy short-circuits everything, but bypass still applies.
  const envProxy = target.isHttps ? (httpsProxy ?? httpProxy) : httpProxy
  if (envProxy) {
    return isBypassed(target.hostname, target.port, patterns)
      ? undefined
      : envProxy
  }

  const run = options.run
  if (platform === 'win32') {
    return resolveWindowsProxy(target, patterns, run, options)
  }
  if (platform === 'darwin') {
    return resolveMacOSProxy(target, patterns, run)
  }
  return undefined
}

async function resolveWindowsProxy(
  target: { hostname: string; port: string; isHttps: boolean },
  basePatterns: string[],
  run: ResolveProxyOptions['run'],
  options: ResolveProxyOptions,
): Promise<string | undefined> {
  const settings = readWindowsProxySettings({ platform: 'win32', run })
  if (!settings) return undefined
  const manual = parseProxyServer(
    settings.enabled ? settings.proxyServer : undefined,
  )
  const patterns = [
    ...basePatterns,
    ...parseProxyOverride(settings.proxyOverride),
  ]
  if (isBypassed(target.hostname, target.port, patterns)) return undefined
  // A PAC script wins over the manual entry when both are configured,
  // matching browser behavior. An explicit DIRECT verdict is final; only
  // a PAC failure falls back to the manual entry.
  if (settings.autoConfigUrl) {
    const targetForPac = `http${target.isHttps ? 's' : ''}://${target.hostname}:${target.port}/`
    const verdict = await (options.evaluatePac?.(
      settings.autoConfigUrl,
      targetForPac,
    ) ?? evaluatePacUrl(settings.autoConfigUrl, targetForPac))
    if (verdict.kind === 'proxy') return verdict.url
    if (verdict.kind === 'direct') return undefined
  }
  const entry = target.isHttps ? (manual.https ?? manual.http) : manual.http
  return entry
}

function resolveMacOSProxy(
  target: { hostname: string; port: string; isHttps: boolean },
  basePatterns: string[],
  run: ResolveProxyOptions['run'],
): string | undefined {
  const settings = readMacOSProxySettings({ platform: 'darwin', run })
  if (!settings) return undefined
  const patterns = [...basePatterns, ...settings.exceptions]
  if (settings.excludeSimpleHostnames) patterns.push('<local>')
  if (isBypassed(target.hostname, target.port, patterns)) return undefined
  return target.isHttps
    ? (settings.httpsProxy ?? settings.httpProxy)
    : (settings.httpProxy ?? settings.httpsProxy)
}
