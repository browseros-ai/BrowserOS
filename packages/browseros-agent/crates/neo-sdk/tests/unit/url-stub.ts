/**
 * The answers `__neo_runtime.get` gives the realm's `URL` (`src/url.ts`), with Bun's WHATWG `URL`
 * standing in for Rust's parser (`url_match.rs`): `parseURL` (with an optional base) and `setURL`.
 * `undefined` for any other request, which the calling stub answers itself.
 */

type Asked = {
  field?: string
  url?: string
  base?: string
  href?: string
  part?: string
  value?: string
}

export function answerURL(asked: Asked): string | undefined {
  if (asked.field === 'parseURL') {
    try {
      return parts(new URL(asked.url as string, asked.base))
    } catch {
      return JSON.stringify(null)
    }
  }
  if (asked.field === 'setURL') {
    const url = new URL(asked.href as string)
    try {
      ;(url as unknown as Record<string, string>)[asked.part as string] =
        asked.value as string
    } catch {
      return JSON.stringify(null)
    }
    return parts(url)
  }
  return undefined
}

function parts(url: URL): string {
  const { href, origin, protocol, username, password, host } = url
  const { hostname, port, pathname, search, hash } = url
  return JSON.stringify({
    href,
    origin,
    protocol,
    username,
    password,
    host,
    hostname,
    port,
    pathname,
    search,
    hash,
    searchParams: [...url.searchParams],
  })
}
