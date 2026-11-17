/** Static half of upstream TestServer. Dynamic routes are deliberately unavailable in both
 * runtimes; emulating them on only the oracle would turn harness differences into Neo verdicts. */

import { readFile } from 'node:fs/promises'
import http from 'node:http'
import { resolve, sep } from 'node:path'
import mime from 'mime'
export async function listen(port, assetsDir) {
  const root = resolve(assetsDir)
  const server = http.createServer(async (req, res) => {
    let file
    try {
      const path = decodeURIComponent(
        new URL(req.url, 'http://localhost').pathname,
      )
      file = resolve(root, `.${path === '/' ? '/index.html' : path}`)
      if (!file.startsWith(root + sep)) throw new Error('outside asset root')
      const body = await readFile(file)
      const type = mime.getType(file) || 'application/octet-stream'
      res.writeHead(200, {
        'Content-Type':
          type +
          (/^(text\/)|javascript|json/.test(type) ? '; charset=utf-8' : ''),
        'Cache-Control': 'no-cache, no-store',
      })
      res.end(body)
    } catch {
      res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' })
      res.end(`File not found: ${file || req.url}`)
    }
  })
  await new Promise((ok, fail) => {
    server.once('error', fail)
    server.listen(port, '::', ok)
  })
  return {
    PREFIX: `http://localhost:${port}`,
    CROSS_PROCESS_PREFIX: `http://127.0.0.1:${port}`,
    EMPTY_PAGE: `http://localhost:${port}/empty.html`,
    PORT: port,
    HOST: 'localhost',
    HOSTNAME: 'localhost',
    close: () => new Promise((r) => server.close(r)),
  }
}
