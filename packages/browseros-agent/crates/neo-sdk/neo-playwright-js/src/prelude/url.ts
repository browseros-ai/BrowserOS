/**
 * URLs without `URL`, which QuickJS lacks.
 *
 * Matching that Playwright does against a page's URL runs in the page instead (Rust's
 * `page.waitForURL`, the probe's vendored `urlMatches`), where `URL` exists and globs resolve
 * exactly as upstream. What must run here is small: a predicate function gets a URL-like object
 * (`urlLike`), a glob tests a cached frame URL (`globMatches`, vendored `globToRegexPattern`),
 * and a string URL an assertion compares with exactly gets the normalizations `new URL()` would
 * apply for an ordinary absolute URL (`normalizeUrl`).
 */
import { globToRegexPattern } from './vendored'

/** The `URL` members predicates read. `searchParams` is a plain lookup, not URLSearchParams. */
export type UrlLike = {
  href: string
  protocol: string
  username: string
  password: string
  host: string
  hostname: string
  port: string
  origin: string
  pathname: string
  search: string
  hash: string
  searchParams: {
    get(name: string): string | null
    has(name: string): boolean
    getAll(name: string): string[]
  }
  toString(): string
}

const DEFAULT_PORTS: Record<string, string> = {
  'http:': '80',
  'https:': '443',
  'ws:': '80',
  'wss:': '443',
  'ftp:': '21',
}
const SPECIAL = new Set(['http:', 'https:', 'ws:', 'wss:', 'ftp:', 'file:'])

const ABSOLUTE =
  /^([a-zA-Z][a-zA-Z0-9+.-]*:)(?:\/\/(?:([^@/?#]*)@)?(\[[^\]]*\]|[^:/?#]*)(?::(\d*))?)?([^?#]*)(\?[^#]*)?(#.*)?$/

function decode(text: string): string {
  try {
    return decodeURIComponent(text.replace(/\+/g, ' '))
  } catch {
    return text
  }
}

/** Parses an absolute URL, or returns null for anything else. */
export function urlLike(href: string): UrlLike | null {
  const match = ABSOLUTE.exec(href)
  if (!match) return null
  const [
    ,
    scheme = '',
    userinfo = '',
    host = '',
    rawPort = '',
    path = '',
    query = '',
    fragment = '',
  ] = match
  const protocol = scheme.toLowerCase()
  const special = SPECIAL.has(protocol)
  const hostname = special ? host.toLowerCase() : host
  const port = rawPort === DEFAULT_PORTS[protocol] ? '' : rawPort
  const pathname = path === '' && special ? '/' : path
  const [username = '', password = ''] = userinfo.split(':')
  const authority = href.slice(protocol.length).startsWith('//')
  const hostPort = port ? `${hostname}:${port}` : hostname
  const credentials = userinfo ? `${userinfo}@` : ''
  const normalized = authority
    ? `${protocol}//${credentials}${hostPort}${pathname}${query}${fragment}`
    : `${protocol}${pathname}${query}${fragment}`
  const search = query === '?' ? '' : query
  const pairs = search
    .slice(1)
    .split('&')
    .filter((pair) => pair !== '')
    .map((pair) => {
      const equals = pair.indexOf('=')
      return equals === -1
        ? [decode(pair), '']
        : [decode(pair.slice(0, equals)), decode(pair.slice(equals + 1))]
    })
  return {
    href: normalized,
    protocol,
    username,
    password,
    host: hostPort,
    hostname,
    port,
    origin:
      special && protocol !== 'file:' ? `${protocol}//${hostPort}` : 'null',
    pathname,
    search,
    hash: fragment === '#' ? '' : fragment,
    searchParams: {
      get: (name) => pairs.find(([key]) => key === name)?.[1] ?? null,
      has: (name) => pairs.some(([key]) => key === name),
      getAll: (name) =>
        pairs.filter(([key]) => key === name).map(([, value]) => value ?? ''),
    },
    toString: () => normalized,
  }
}

/** `new URL(text).href` for an ordinary absolute URL; any other text is returned unchanged. */
export function normalizeUrl(text: string): string {
  return urlLike(text)?.href ?? text
}

/** Playwright's glob test, for URLs the prelude already holds (`page.frame({ url })`). */
export function globMatches(glob: string, url: string): boolean {
  if (glob === '') return true
  return new RegExp(globToRegexPattern(glob)).test(url)
}
