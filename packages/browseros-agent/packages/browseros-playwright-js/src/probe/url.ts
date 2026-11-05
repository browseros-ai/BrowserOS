/**
 * `page.waitForURL`'s pattern, matched here because a RegExp has to run with JavaScript's
 * semantics, which Rust's regex crate does not share. A glob goes through the vendored
 * `urlMatches`, as upstream's client does; Neo has no `baseURL`.
 */
import { urlMatches } from '../../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/urlMatch'
import type { MatchUrlRequest, UrlMatch } from './protocol'
import { ProbeError, type World } from './world'

export function matchUrl(_world: World, request: MatchUrlRequest): UrlMatch {
  const { pattern, url } = request
  const match =
    'glob' in pattern
      ? pattern.glob
      : regExp(pattern.regexSource, pattern.regexFlags)
  return { kind: 'urlMatch', matches: urlMatches(undefined, url, match), url }
}

/** The script's RegExp; one this engine cannot compile is the script's error. */
function regExp(source: string, flags: string): RegExp {
  try {
    return new RegExp(source, flags)
  } catch (error) {
    throw new ProbeError(
      'script',
      error instanceof Error ? error.message : String(error),
    )
  }
}
