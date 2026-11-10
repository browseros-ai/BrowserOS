/**
 * The one fixture server both conformance runners load pages from, so Runner P and Runner N
 * see the same bytes at the same URLs. It listens on `[::]`, dual-stack, so `127.0.0.1`,
 * `localhost` and `[::1]` all reach it and Chromium treats them as three sites: a page on one
 * with an iframe on another gets an out-of-process iframe.
 *
 *   /<file>          conformance/pages/<file>
 *   /nest/<file>     tests/data/nest/<file>, the nested-frame fixture of design D22
 *   /upstream/<file> third_party/playwright/tests/assets/<file>, the pages of upstream's test
 *                    server that tier 2's lifted tests load (its server.PREFIX)
 *
 * Two query parameters shape any response, for the cases that time load states:
 *   ?delay=<ms>      answer after <ms>
 *   ?redirect=<url>  answer 302 to <url>
 * Every response is `cache-control: no-store`, so a reload or history step refetches.
 *
 *   node server.mjs [port]   prints `listening <port>` once it accepts connections
 */
import { readFile } from 'node:fs/promises'
import http from 'node:http'
import { dirname, extname, normalize, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const ROOTS = [
  ['/nest/', resolve(here, '../data/nest')],
  ['/upstream/', resolve(here, '../../third_party/playwright/tests/assets')],
  ['/', resolve(here, 'pages')],
]
const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.txt': 'text/plain; charset=utf-8',
}

/** The file a request path names, or null when it is outside every root. */
function fileFor(pathname) {
  for (const [prefix, root] of ROOTS) {
    if (!pathname.startsWith(prefix)) continue
    const file = resolve(root, normalize(pathname.slice(prefix.length)))
    return file.startsWith(`${root}/`) ? file : null
  }
  return null
}

async function answer(request, response) {
  const url = new URL(request.url, 'http://fixture')
  const delay = Number(url.searchParams.get('delay') ?? 0)
  if (delay > 0) await new Promise((done) => setTimeout(done, delay))
  const redirect = url.searchParams.get('redirect')
  if (redirect !== null) {
    response.writeHead(302, { location: redirect, 'cache-control': 'no-store' })
    response.end()
    return
  }
  const file = fileFor(url.pathname)
  const body = file === null ? null : await readFile(file).catch(() => null)
  if (body === null) {
    response.writeHead(404, {
      'content-type': TYPES['.txt'],
      'cache-control': 'no-store',
    })
    response.end('not found')
    return
  }
  response.writeHead(200, {
    'content-type': TYPES[extname(file)] ?? 'application/octet-stream',
    'cache-control': 'no-store',
  })
  response.end(body)
}

const server = http.createServer((request, response) => {
  answer(request, response).catch((error) => {
    response.writeHead(500).end(String(error))
  })
})
server.listen(Number(process.argv[2] ?? 0), '::', () => {
  console.log(`listening ${server.address().port}`)
})
