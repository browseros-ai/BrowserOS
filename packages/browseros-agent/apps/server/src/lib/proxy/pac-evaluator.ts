/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * PAC (Proxy Auto-Config) evaluation for Windows AutoConfigURL / manual setup
 * scripts.
 *
 * The PAC script is fetched directly (never through a proxy — that would be a
 * chicken-and-egg loop), evaluated per target URL with pac-resolver inside a
 * QuickJS WASM sandbox, and the winning directive is mapped to a Bun fetch
 * `proxy` URL. SOCKS directives fall back to DIRECT: Bun's fetch proxy is
 * HTTP(S)-only. Every failure path fails open to DIRECT with a log line.
 */

import { createPacResolver } from 'pac-resolver'
import { QuickJS } from 'quickjs-wasi'
import { logger } from '../logger'

const PAC_TEXT_TTL_MS = 5 * 60 * 1000

export type PacDirective =
  | { type: 'proxy'; url: string }
  | { type: 'direct' }
  | { type: 'unsupported' }

/** Outcome of PAC evaluation. `failed` means the PAC could not be used at all. */
export type PacVerdict =
  | { kind: 'proxy'; url: string }
  | { kind: 'direct' }
  | { kind: 'failed' }

/**
 * Parse a FindProxyForURL result. First directive wins per the PAC spec.
 * Exported for tests.
 */
export function parsePacResult(result: string): PacDirective {
  const first = result.split(';')[0]?.trim() ?? ''
  const space = first.indexOf(' ')
  const keyword = (space === -1 ? first : first.slice(0, space)).toUpperCase()
  const host = space === -1 ? '' : first.slice(space + 1).trim()
  if (keyword === 'PROXY' && host)
    return { type: 'proxy', url: withScheme(host) }
  if (keyword === 'HTTPS' && host) {
    return { type: 'proxy', url: withScheme(host, 'https') }
  }
  if (keyword === 'SOCKS' || keyword === 'SOCKS5' || keyword === 'SOCKS4') {
    return { type: 'unsupported' }
  }
  return { type: 'direct' }
}

function withScheme(host: string, scheme = 'http'): string {
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(host)) return host
  return `${scheme}://${host}`
}

export type PacResolverFn = (url: string) => Promise<string>

export interface PacEvaluatorDeps {
  loadResolver?: (pacText: string) => Promise<PacResolverFn>
  fetchText?: (url: string) => Promise<string | undefined>
}

const pacTextCache = new Map<string, { text: string; fetchedAt: number }>()
let runtime: QuickJS | undefined

/** Reset module caches. Test-only. */
export function resetPacCache(): void {
  pacTextCache.clear()
  runtime = undefined
}

async function defaultLoadResolver(pacText: string): Promise<PacResolverFn> {
  // The quickjs.wasm blob is read from node_modules at runtime. That fails
  // inside the compiled production binary (bundled JS, no sibling wasm file),
  // so PAC degrades to DIRECT there until the wasm ships as a prod resource.
  // Manual proxy and env vars are unaffected.
  runtime ??= await QuickJS.create()
  return createPacResolver(runtime, pacText)
}

async function defaultFetchText(url: string): Promise<string | undefined> {
  const response = await globalThis.fetch(url)
  if (!response.ok) {
    logger.warn('Failed to fetch PAC script', {
      url,
      status: response.status,
    })
    return undefined
  }
  return response.text()
}

export async function evaluatePacText(
  pacText: string,
  targetUrl: string,
  deps: PacEvaluatorDeps = {},
): Promise<PacVerdict> {
  let resolver: PacResolverFn
  try {
    resolver = await (deps.loadResolver?.(pacText) ??
      defaultLoadResolver(pacText))
  } catch (err) {
    logger.warn('PAC runtime unavailable, connecting directly', {
      error: err instanceof Error ? err.message : String(err),
    })
    return { kind: 'failed' }
  }
  let result: string
  try {
    result = await resolver(targetUrl)
  } catch (err) {
    logger.debug('PAC evaluation failed, connecting directly', {
      targetUrl,
      error: err instanceof Error ? err.message : String(err),
    })
    return { kind: 'failed' }
  }
  const directive = parsePacResult(result)
  if (directive.type === 'proxy') return { kind: 'proxy', url: directive.url }
  if (directive.type === 'unsupported') {
    logger.warn('PAC returned a SOCKS proxy, connecting directly', {
      targetUrl,
      result,
    })
    return { kind: 'failed' }
  }
  return { kind: 'direct' }
}

export async function evaluatePacUrl(
  pacUrl: string,
  targetUrl: string,
  deps: PacEvaluatorDeps = {},
): Promise<PacVerdict> {
  const cached = pacTextCache.get(pacUrl)
  let pacText =
    cached && Date.now() - cached.fetchedAt < PAC_TEXT_TTL_MS
      ? cached.text
      : undefined
  if (!pacText) {
    try {
      pacText = await (deps.fetchText?.(pacUrl) ?? defaultFetchText(pacUrl))
    } catch (err) {
      logger.debug('PAC fetch failed, connecting directly', {
        pacUrl,
        error: err instanceof Error ? err.message : String(err),
      })
      return { kind: 'failed' }
    }
    if (!pacText) return { kind: 'failed' }
    pacTextCache.set(pacUrl, { text: pacText, fetchedAt: Date.now() })
  }
  return evaluatePacText(pacText, targetUrl, deps)
}
