/**
 * Writes `fixtures/url-match-goldens.json`: what Playwright's own `urlMatches` and
 * `resolveGlobToRegexPattern` (`packages/isomorphic/urlMatch.ts` at 26137face) answer for each
 * case below, with no `baseURL`, as `waitForURL` calls them. `src/url_match.rs` ports that code to
 * Rust and evaluates the patterns with QuickJS; its `tests_that_goldens_match` replays this table,
 * so a port that drifts from Playwright fails there.
 *
 * Run it with Node, the runtime Playwright's client (where `waitForURL` matches) runs in, from
 * `packages/browseros-agent`:
 *
 *   node crates/neo-sdk/neo-runtime/tests/url-match-goldens.mjs          # rewrite the table
 *   node crates/neo-sdk/neo-runtime/tests/url-match-goldens.mjs --check  # fail if it is stale
 *
 * It imports the TypeScript source from a read-only Playwright checkout, `--playwright=<dir>`
 * (default below), and refuses any checkout that is not at 26137face or has local edits there.
 */

import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { register } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const PINNED = '26137face'
const SOURCE = 'packages/isomorphic/urlMatch.ts'
const DEFAULT_PLAYWRIGHT = '/Volumes/extssd/ext-workspaces/ref/playwright'
const OUTPUT = join(
  dirname(fileURLToPath(import.meta.url)),
  'fixtures',
  'url-match-goldens.json',
)

/** Globs, each with the URLs to match it against. */
const GLOBS = [
  // `''` matches every URL before any pattern is built.
  ['', ['http://a.test/', 'about:blank', 'data:text/html,x']],
  ['**', ['http://a.test/', 'http://a.test/a/b?c#d', 'about:blank']],
  ['*', ['http://a.test/', 'about:blank', 'a.test']],
  ['**/*', ['http://a.test/', 'http://a.test/a/b', 'about:blank']],
  [
    '**/done.html',
    [
      'http://a.test/done.html',
      'http://a.test/x/y/done.html',
      'http://a.test/undone.html',
      'http://a.test/done.html?x=1',
      'http://a.test/done.html#top',
    ],
  ],
  ['**/done.html*', ['http://a.test/done.html?x=1', 'http://a.test/done.htm']],
  ['*/done.html', ['http://a.test/done.html', 'a.test/done.html']],
  [
    'http://a.test/**',
    ['http://a.test/', 'http://a.test/a/b/c', 'http://b.test/a'],
  ],
  ['http://a.test/*', ['http://a.test/a', 'http://a.test/a/b']],
  [
    '**/a/**/b',
    ['http://a.test/a/b', 'http://a.test/a/x/y/b', 'http://a.test/ab'],
  ],
  ['**/a**', ['http://a.test/a', 'http://a.test/ab/c']],
  // `{a,b}` alternation, and commas outside a group.
  [
    '**/{a,b}.html',
    ['http://a.test/a.html', 'http://a.test/b.html', 'http://a.test/c.html'],
  ],
  [
    'http://a.test/{x,y}/*',
    ['http://a.test/x/1', 'http://a.test/y/2', 'http://a.test/z/3'],
  ],
  ['http://a.test/{a,b/c}', ['http://a.test/a', 'http://a.test/b/c']],
  ['**/{,index.html}', ['http://a.test/', 'http://a.test/index.html']],
  ['**/a,b', ['http://a.test/a,b', 'http://a.test/a']],
  ['**/{}', ['http://a.test/']],
  // Invalid globs.
  ['{a', []],
  ['a}', []],
  ['**/{a,{b}}', []],
  ['http://a.test/{a', []],
  // `?` is a literal character in Playwright's globs, not a wildcard.
  [
    '**/search?q=*',
    [
      'http://a.test/search?q=neo',
      'http://a.test/searchXq=neo',
      'http://a.test/search?q=a/b',
    ],
  ],
  [
    'http://a.test/search?q=1',
    ['http://a.test/search?q=1', 'http://a.test/searchXq=1'],
  ],
  ['http://a.test/?q=*', ['http://a.test/?q=x', 'http://a.test/?r=x']],
  ['http://a.test/a?', ['http://a.test/a?', 'http://a.test/a']],
  ['http://a.test/page\\\\?x', ['http://a.test/page?x']],
  ['**/page\\\\?x', ['http://a.test/page\\?x', 'http://a.test/page?x']],
  // Escaped glob characters are literal; regex metacharacters are always literal.
  ['**/a\\*b', ['http://a.test/a*b', 'http://a.test/axb']],
  ['**/\\{a\\}', ['http://a.test/{a}', 'http://a.test/a']],
  ['**/a\\,b', ['http://a.test/a,b']],
  ['**/a\\d', ['http://a.test/ad', 'http://a.test/a1']],
  ['**/a\\', ['http://a.test/a\\']],
  ['**/a.b', ['http://a.test/a.b', 'http://a.test/aXb']],
  [
    '**/x+(y)|[z]$^',
    ['http://a.test/x+(y)|[z]$^', 'http://a.test/xx(y)|[z]$^'],
  ],
  // Literal URL parts round-trip through the WHATWG URL parser.
  ['http://a.test', ['http://a.test/', 'http://a.test']],
  ['https://a.test', ['https://a.test/', 'http://a.test/']],
  ['http://a.test/path', ['http://a.test/path', 'http://a.test/path/']],
  ['http://a.test/path/', ['http://a.test/path/', 'http://a.test/path']],
  ['http://a.test:80/x', ['http://a.test/x', 'http://a.test:80/x']],
  ['https://a.test:443/', ['https://a.test/']],
  ['http://a.test:443/', ['http://a.test:443/', 'http://a.test/']],
  ['http://a.test:8080/*', ['http://a.test:8080/a', 'http://a.test/a']],
  ['http://localhost:*/a', ['http://localhost:3000/a', 'http://localhost/a']],
  ['http://a.test/a/../b', ['http://a.test/b', 'http://a.test/a/../b']],
  ['http://a.test/./x', ['http://a.test/x']],
  ['http://a.test/a b', ['http://a.test/a%20b', 'http://a.test/a b']],
  ['http://a.test/é', ['http://a.test/%C3%A9']],
  ['http://bücher.test/', ['http://xn--bcher-kva.test/']],
  // Case: the scheme and host are case-insensitive, the rest is not.
  ['HTTP://A.TEST/Path', ['http://a.test/Path', 'http://a.test/path']],
  [
    'http://*.EXAMPLE.com/',
    ['http://www.example.com/', 'http://www.EXAMPLE.com/'],
  ],
  ['http://A.test/{X,Y}', ['http://a.test/X', 'http://a.test/x']],
  ['http*://a.test/', ['http://a.test/', 'https://a.test/', 'ws://a.test/']],
  ['*://a.test/', ['http://a.test/', 'https://a.test/']],
  ['http://*/x', ['http://a.test/x', 'http://a.test/y/x']],
  // Other schemes: special (`ws:`), and opaque-origin ones whose case is kept.
  ['WS://A.test/*', ['ws://a.test/x', 'WS://A.test/x']],
  ['blob:HTTP://A.test/*', ['blob:http://a.test/x', 'blob:HTTP://A.test/x']],
  ['mailto:A@B.test', ['mailto:A@B.test', 'mailto:a@b.test']],
  ['javascript:void(0)', ['javascript:void(0)']],
  // These prefixes skip URL normalization.
  ['about:blank', ['about:blank', 'about:blank#x', 'about:BLANK']],
  ['about:*', ['about:blank', 'about:srcdoc']],
  ['data:text/html,**', ['data:text/html,<p>hi</p>', 'data:text/plain,x']],
  ['file:///tmp/*.html', ['file:///tmp/a.html', 'file:///tmp/a/b.html']],
  ['chrome://newtab/', ['chrome://newtab/']],
  // Hash fragments are part of the URL.
  [
    'http://a.test/page#section',
    ['http://a.test/page#section', 'http://a.test/page'],
  ],
  ['**/page#*', ['http://a.test/page#x', 'http://a.test/page']],
  ['http://a.test/page', ['http://a.test/page', 'http://a.test/page#x']],
  // Without a base URL a relative glob never parses as a URL and stays as written.
  ['/done.html', ['http://a.test/done.html', '/done.html']],
  ['done.html', ['done.html']],
  ['./a/*', ['./a/b']],
  // A replacement string's `$&` stays literal, and the first placeholder occurrence wins.
  ['http://a.test/$&*', ['http://a.test/$&x', 'http://a.test/x']],
  ['http://a.test/$_4_$/x*', ['http://a.test/x*/$_4_$', 'http://a.test/x']],
  // Non-ASCII glob text.
  ['**/ü*', ['http://a.test/ü', 'http://a.test/%C3%BC']],
  ['**/😀*', ['http://a.test/😀x']],
]

/** RegExps, each with the URLs to test it against, in order, on the same object. */
const REGEXES = [
  [/done\.html$/, ['http://a.test/done.html', 'http://a.test/done.htm']],
  [/DONE/i, ['http://a.test/done', 'http://a.test/x']],
  // `lastIndex` is reset before each test, so a global RegExp matches twice in a row.
  [/done/g, ['http://a.test/done', 'http://a.test/done', 'http://a.test/x']],
  [/done/y, ['http://a.test/done', 'done']],
  [/http/y, ['http://a.test/']],
  [/done/gi, ['http://a.test/DONE', 'http://a.test/Done']],
  [/^about:blank$/, ['about:blank', 'about:blank#x']],
  [/#section$/, ['http://a.test/page#section', 'http://a.test/page']],
  [/^http:\/\/a\.test\/$/m, ['http://a.test/']],
  [/(?<word>o)\k<word>/, ['http://a.test/foo', 'http://a.test/fo']],
  [/a(?=\.test)/, ['http://a.test/']],
  [/(?<=a\.)test/, ['http://a.test/', 'http://b.test/']],
  [/\p{Lu}/u, ['http://a.test/X', 'http://a.test/x']],
  [/./s, ['about:blank']],
  [/test/d, ['http://a.test/']],
  [/(?:)/, ['http://a.test/', '']],
  [/a\/b/, ['http://a.test/a/b']],
]

const args = process.argv.slice(2)
const check = args.includes('--check')
const playwright =
  args.find((arg) => arg.startsWith('--playwright='))?.slice(13) ??
  DEFAULT_PLAYWRIGHT

assertPinned(playwright)
const { resolveGlobToRegexPattern, urlMatches } = await importUrlMatch(
  join(playwright, SOURCE),
)

const goldens = {
  about: `Playwright ${PINNED} ${SOURCE}, no baseURL. Written by url-match-goldens.mjs; do not edit.`,
  globs: GLOBS.map(([glob, urls]) => {
    let pattern
    try {
      pattern = resolveGlobToRegexPattern(undefined, glob)
    } catch (error) {
      return { glob, error: error.message }
    }
    return {
      glob,
      pattern,
      urls: urls.map((url) => [url, urlMatches(undefined, url, glob)]),
    }
  }),
  regexes: REGEXES.map(([regex, urls]) => ({
    source: regex.source,
    flags: regex.flags,
    urls: urls.map((url) => [url, urlMatches(undefined, url, regex)]),
  })),
}
const text = `${JSON.stringify(goldens, null, 2)}\n`

const counts = `${goldens.globs.length} globs, ${goldens.regexes.length} RegExps`
if (check) {
  // Compared as parsed JSON: the repo's formatter rewrites the committed file's layout, never
  // its values.
  let committed
  try {
    committed = JSON.parse(readFileSync(OUTPUT, 'utf8'))
  } catch {}
  if (JSON.stringify(committed) !== JSON.stringify(goldens)) {
    console.error(`${OUTPUT} is stale: rerun without --check and commit it.`)
    process.exit(1)
  }
  console.log(`url-match-goldens: ${counts} match`)
} else {
  writeFileSync(OUTPUT, text)
  console.log(`wrote ${OUTPUT}: ${counts}`)
}

function assertPinned(repo) {
  const git = (...gitArgs) =>
    execFileSync('git', ['-C', repo, ...gitArgs], { encoding: 'utf8' }).trim()
  const head = git('rev-parse', 'HEAD')
  if (!head.startsWith(PINNED))
    throw new Error(`${repo} is at ${head}, not Playwright ${PINNED}`)
  if (git('status', '--porcelain', '--', 'packages/isomorphic'))
    throw new Error(`${repo} has local edits under packages/isomorphic`)
}

/** Imports the TypeScript module. Node strips its types; its extensionless relative imports
 * (`./stringUtils`) need a resolve hook, which Bun does not. */
async function importUrlMatch(path) {
  if (!process.versions.bun) {
    const hook = `export async function resolve(specifier, context, next) {
      const ts = specifier.startsWith('.') && !specifier.endsWith('.ts')
      const resolved = await next(ts ? specifier + '.ts' : specifier, context)
      return resolved.url.endsWith('.ts') ? { ...resolved, format: 'module-typescript' } : resolved
    }`
    register(`data:text/javascript,${encodeURIComponent(hook)}`)
  }
  return await import(path)
}
