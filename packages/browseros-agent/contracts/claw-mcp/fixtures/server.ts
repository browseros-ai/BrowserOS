/**
 * Static fixture server for the claw-mcp contract suite. Serves the
 * deterministic pages in `fixtures/pages/` from an in-process Bun
 * server; `startFixturePair` boots two instances so `iframe.html` can
 * embed a genuinely cross-origin child frame (a real OOPIF).
 *
 * Ports are drawn from 10101-20202: everything below 10101 risks a
 * Chromium restricted port (net/base/port_util.cc kRestrictedPorts),
 * which the browser refuses to navigate to.
 */

import { readdir } from 'node:fs/promises'
import { resolve } from 'node:path'

const FIXTURE_PORT_MIN = 10101
const FIXTURE_PORT_MAX = 20202
const PAGES_DIR = resolve(import.meta.dir, 'pages')

export interface FixtureServer {
  port: number
  origin: string
  url(path: string): string
  stop(): Promise<void>
}

export interface FixturePair {
  primary: FixtureServer
  secondary: FixtureServer
  stop(): Promise<void>
}

function randomFixturePort(): number {
  const span = FIXTURE_PORT_MAX - FIXTURE_PORT_MIN + 1
  return FIXTURE_PORT_MIN + Math.floor(Math.random() * span)
}

async function handleRequest(request: Request): Promise<Response> {
  const { pathname } = new URL(request.url)
  if (pathname === '/files/report.txt') {
    return new Response('fixture report contents\n', {
      headers: {
        'content-type': 'text/plain',
        'content-disposition': 'attachment; filename="report.txt"',
      },
    })
  }
  if (/^\/[\w-]+\.html$/.test(pathname)) {
    const file = Bun.file(resolve(PAGES_DIR, pathname.slice(1)))
    if (await file.exists()) {
      return new Response(file, {
        headers: { 'content-type': 'text/html; charset=utf-8' },
      })
    }
  }
  return new Response('not found', { status: 404 })
}

export async function startFixtureServer(): Promise<FixtureServer> {
  let lastError: unknown
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const port = randomFixturePort()
    try {
      const server = Bun.serve({
        hostname: '127.0.0.1',
        port,
        fetch: handleRequest,
      })
      const origin = `http://127.0.0.1:${port}`
      return {
        port,
        origin,
        url: (path) => `${origin}${path.startsWith('/') ? path : `/${path}`}`,
        stop: async () => {
          await server.stop(true)
        },
      }
    } catch (error) {
      lastError = error
    }
  }
  throw new Error(
    `could not bind a fixture port after 20 attempts: ${lastError}`,
  )
}

export async function startFixturePair(): Promise<FixturePair> {
  const primary = await startFixtureServer()
  const secondary = await startFixtureServer()
  return {
    primary,
    secondary,
    stop: async () => {
      await Promise.all([primary.stop(), secondary.stop()])
    },
  }
}

export async function listFixturePages(): Promise<string[]> {
  const entries = await readdir(PAGES_DIR)
  return entries.filter((entry) => entry.endsWith('.html')).sort()
}
