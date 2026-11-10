/**
 * Records the snapshot fixture from real Playwright, so the crate's stitching is tested on what
 * Playwright's own engine prints for each frame rather than on hand-written trees:
 *
 *   crates/neo-sdk/neo-playwright/tests/data/snapshot-frames.json
 *       frames    each frame's own ariaSnapshotJSON in ai mode, with the iframes it printed
 *                 and which frame each one holds (by index into frames)
 *       stitched  upstream's stitched tree for the page (page.ariaSnapshotJSON)
 *       yaml      upstream's text for the page (page.ariaSnapshot)
 *
 * The pages are crates/neo-sdk/neo-playwright/tests/data/snapshot/, served on three sites so the
 * child frames are out-of-process iframes. Playwright returns a frame's tree only with its
 * child frames already stitched in (locator('body').ariaSnapshotJSON in that frame), so the
 * script lifts each child's tree back out of its iframe node, checks it equals that child's
 * own capture, and checks that stitching the lifted frames again gives upstream's tree.
 *
 *   PLAYWRIGHT_NODE_MODULES=<node_modules with playwright-core 1.64.1> \
 *   BROWSEROS_CDP_PORT=9421 node scripts/capture-snapshot-fixture.mjs [--check]
 *
 * --check recaptures and fails if the file would change. The script opens and closes its own
 * tab and never touches another.
 */
import { existsSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { isDeepStrictEqual } from 'node:util'

const here = dirname(fileURLToPath(import.meta.url))
const agentRoot = resolve(here, '../../../..')
const dataRoot = resolve(agentRoot, 'crates/neo-sdk/neo-playwright/tests/data')
const pagesRoot = join(dataRoot, 'snapshot')
const output = join(dataRoot, 'snapshot-frames.json')
const vendoredTag = readFileSync(
  resolve(
    agentRoot,
    'crates/neo-sdk/neo-playwright/third_party/playwright/VERSION',
  ),
  'utf8',
).split('\n')[0]
const nodeModules = process.env.PLAYWRIGHT_NODE_MODULES
if (!nodeModules) {
  console.error(
    'Set PLAYWRIGHT_NODE_MODULES to a node_modules holding playwright-core',
  )
  process.exit(2)
}
const require = createRequire(
  resolve(nodeModules, 'capture-snapshot-fixture.cjs'),
)
const { version } = require('playwright-core/package.json')
if (`v${version}` !== vendoredTag) {
  console.error(
    `playwright-core is ${version}; the vendored engine is ${vendoredTag}`,
  )
  process.exit(2)
}
const { chromium } = require('playwright-core')
const CDP_PORT = Number(process.env.BROWSEROS_CDP_PORT ?? 9005)
// Fixed, because frame URLs are part of the fixture.
const PAGES_PORT = 42051
const FRAMES = 4

/** Serves the fixture pages on [::]:PAGES_PORT, which 127.0.0.1, localhost and [::1] all reach. */
function servePages() {
  const server = http.createServer((request, response) => {
    const file = join(dataRoot, request.url.split('?')[0])
    if (
      !file.startsWith(pagesRoot) ||
      !existsSync(file) ||
      statSync(file).isDirectory()
    ) {
      response.writeHead(404).end()
      return
    }
    response.writeHead(200, { 'content-type': 'text/html' })
    response.end(readFileSync(file))
  })
  return new Promise((listening) =>
    server.listen(PAGES_PORT, '::', () => listening(server)),
  )
}

/**
 * Takes each child frame's tree back out of the iframe node upstream put it under, and returns
 * the frame's own nodes and the iframes it printed. Like upstream's merge, it stops at iframe
 * nodes that have a ref and descends into everything else.
 */
function lift(stitched, children) {
  const json = structuredClone(stitched)
  const iframes = []
  const visit = (node, depth) => {
    if (typeof node === 'string') return
    if (node.role === 'iframe' && typeof node.ref === 'string') {
      const child = children.find(({ tree }) =>
        isDeepStrictEqual(tree, node.children),
      )
      if (!child)
        throw new Error(`iframe ${node.ref} holds no captured child frame`)
      iframes.push({ ref: node.ref, depth, src: child.url, frame: child.index })
      delete node.children
      return
    }
    for (const item of node.children ?? []) visit(item, depth + 1)
  }
  for (const node of json) visit(node, 0)
  return { json, iframes }
}

/** Upstream's merge over the lifted frames, to check the lift is exact. */
function restitch(frames, index) {
  const { json, iframes } = frames[index]
  const tree = structuredClone(json)
  const visit = (node) => {
    if (typeof node === 'string') return
    if (node.role === 'iframe' && typeof node.ref === 'string') {
      const iframe = iframes.find(({ ref }) => ref === node.ref)
      const child = iframe ? restitch(frames, iframe.frame) : []
      if (child.length) node.children = child
      return
    }
    for (const item of node.children ?? []) visit(item)
  }
  for (const node of tree) visit(node)
  return tree
}

async function capture() {
  const server = await servePages()
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`)
  const page = await browser.contexts()[0].newPage()
  try {
    // The top page's load waits for its iframes, and each child's for its own.
    await page.goto(`http://127.0.0.1:${PAGES_PORT}/snapshot/top.html`)
    for (let tries = 0; page.frames().length < FRAMES && tries < 100; tries++)
      await page.waitForTimeout(100)
    if (page.frames().length !== FRAMES)
      throw new Error(
        `expected ${FRAMES} frames, found ${page.frames().length}`,
      )
    const frames = page.frames()
    const trees = []
    for (const frame of frames)
      trees.push(await frame.locator('body').ariaSnapshotJSON({ mode: 'ai' }))
    const lifted = frames.map((frame, index) => ({
      url: frame.url(),
      ...lift(
        trees[index],
        frame.childFrames().map((child) => ({
          index: frames.indexOf(child),
          url: child.url(),
          tree: trees[frames.indexOf(child)],
        })),
      ),
    }))
    const stitched = await page.ariaSnapshotJSON({ mode: 'ai' })
    if (!isDeepStrictEqual(restitch(lifted, 0), stitched))
      throw new Error(
        'the lifted frames do not stitch back into upstream’s tree',
      )
    return {
      playwright: vendoredTag.slice(1),
      capture:
        'scripts/capture-snapshot-fixture.mjs: playwright-core over CDP on a throwaway BrowserOS',
      note: 'frames[0] is the page; each iframe names the frame it holds by index into frames.',
      frames: lifted,
      stitched,
      yaml: await page.ariaSnapshot({ mode: 'ai' }),
    }
  } finally {
    await page.close()
    await browser.close()
    server.close()
  }
}

const text = `${JSON.stringify(await capture(), null, 2)}\n`
if (process.argv.includes('--check')) {
  if (readFileSync(output, 'utf8') !== text) {
    console.error(`A recapture differs from ${output}`)
    process.exit(1)
  }
  console.log('matched the snapshot fixture')
} else {
  writeFileSync(output, text)
  console.log(`wrote ${output}`)
}
