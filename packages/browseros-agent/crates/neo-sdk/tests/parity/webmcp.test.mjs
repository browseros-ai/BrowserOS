// `page.webmcp` and `frame.webmcp` (`tools`, `callTool`) through Neo and through Playwright on
// one BrowserOS neo launched with `--enable-features=WebMCP`: the tools a document registered,
// with their schema and annotations; a call's answer as the tool gave it (an `isError` one
// included); a throwing tool, an unknown name and a tool that never settles fail; a navigation
// drops the old document's tools; a frame lists only its own. On a BrowserOS launched without
// the feature, a page has no tools and a call fails naming the missing tool, in both engines.
//
// Playwright cases (pin 26137face): `tests/library/webmcp.spec.ts` "WM-001 should list tools
// registered by the page", "WM-002 should report no tools when the page registers none", "WM-003
// should call a tool", "WM-004 should return isError results as is", "WM-005 should throw when
// the tool throws", "WM-006 should throw for an unknown tool", "WM-007 should time out when the
// tool does not settle" and "WM-008 should drop tools after navigation".

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  openTab,
  runCleanups,
  runInNeo,
  serveFixture,
  skip,
} from './harness.mjs'

const register = (tools) =>
  `<title>WebMCP</title><script>const modelContext = document.modelContext || navigator.modelContext;\nif (modelContext) {${tools}}</script>`

const ADD = `modelContext.registerTool({
  name: 'add',
  description: 'Adds two numbers',
  inputSchema: { type: 'object', properties: { a: { type: 'number' }, b: { type: 'number' } }, required: ['a', 'b'] },
  annotations: { readOnlyHint: true },
  async execute(input) { return { content: [{ type: 'text', text: String(input.a + input.b) }] } },
});`

const pages = {
  '/tools.html': register(
    `${ADD}
modelContext.registerTool({ name: 'noop', description: 'Does nothing', async execute() {} });
modelContext.registerTool({ name: 'broken', description: 'Always fails', async execute() { return { content: [{ type: 'text', text: 'nope' }], isError: true } } });
modelContext.registerTool({ name: 'throwing', description: 'Throws', async execute() { throw new Error('boom') } });
modelContext.registerTool({ name: 'stuck', description: 'Never resolves', execute() { return new Promise(() => {}) } });`,
  ),
  '/framed.html': register(ADD) + '<iframe src="/child.html"></iframe>',
  '/child.html': register(
    `modelContext.registerTool({ name: 'child', description: 'In the frame', async execute() { return 'from the frame' } });`,
  ),
  '/empty.html': '<title>empty</title>',
}

const SOURCE = `const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
const firstLine = (error) => [error.name, error.message.split('\\n')[0]]
await page.goto(origin + '/tools.html')
const tools = (await page.webmcp.tools()).sort((a, b) => a.name.localeCompare(b.name))
const added = await page.webmcp.callTool('add', { a: 2, b: 40 })
const broken = await page.webmcp.callTool('broken')
const throwing = await page.webmcp.callTool('throwing').catch((error) => error)
const missing = await page.webmcp.callTool('missing').then(() => 'called', firstLine)
const stuck = await page.webmcp.callTool('stuck', {}, { timeout: 500 }).catch((error) => error)
await page.goto(origin + '/framed.html')
await page.waitForLoadState('load')
const top = (await page.webmcp.tools()).map((tool) => tool.name)
const child = page.frames().find((frame) => frame.url().endsWith('/child.html'))
const inFrame = (await child.webmcp.tools()).map((tool) => tool.name)
const fromFrame = await child.webmcp.callTool('child')
await page.goto(origin + '/empty.html')
const after = await page.webmcp.tools()
const dropped = await page.webmcp.callTool('add', { a: 1, b: 1 }).then(() => 'called', firstLine)
return {
  tools,
  added,
  broken,
  throwing: [throwing.name, /the invocation failed|Tool execution failed/.test(throwing.message)],
  missing,
  stuck: [stuck.name, stuck.message.includes('Timeout 500ms exceeded')],
  top,
  inFrame,
  fromFrame,
  after,
  dropped,
}`

const EXPECTED = {
  tools: [
    {
      name: 'add',
      description: 'Adds two numbers',
      inputSchema: {
        type: 'object',
        properties: { a: { type: 'number' }, b: { type: 'number' } },
        required: ['a', 'b'],
      },
      annotations: { readOnly: true },
    },
    { name: 'broken', description: 'Always fails' },
    { name: 'noop', description: 'Does nothing' },
    { name: 'stuck', description: 'Never resolves' },
    { name: 'throwing', description: 'Throws' },
  ],
  added: { content: [{ type: 'text', text: '42' }] },
  broken: { content: [{ type: 'text', text: 'nope' }], isError: true },
  throwing: ['Error', true],
  missing: [
    'Error',
    'webMCP.callTool: No WebMCP tool named "missing". Available tools: add, broken, noop, stuck, throwing.',
  ],
  stuck: ['TimeoutError', true],
  top: ['add'],
  inFrame: ['child'],
  fromFrame: 'from the frame',
  after: [],
  dropped: [
    'Error',
    'webMCP.callTool: No WebMCP tool named "add". The frame does not register any WebMCP tools.',
  ],
}

/** Runs `source` through Playwright and through Neo on a browser launched with `args`. */
async function bothEngines(t, args, source) {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages })
    const browser = await launchBrowser(cleanups, browserBinary, args)
    const cdp = await connectCdp(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-webmcp-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close().catch(() => {}))
    const [context] = playwright.contexts()
    const page = await context.newPage()
    await page.goto(`${origin}/empty.html`)
    const AsyncFunction = (async () => {}).constructor
    const reference = await new AsyncFunction('page', 'context', source)(
      page,
      context,
    ).then(
      (value) => ({ value }),
      (error) => ({ error: { name: error.name, message: error.message } }),
    )
    await page.close()
    const { targetId } = await openTab(cdp, `${origin}/empty.html`)
    const file = join(scripts, 'webmcp.js')
    writeFileSync(file, source)
    const neo = await runInNeo(neoRun, browser.port, targetId, file)
    t.diagnostic(JSON.stringify({ reference, neo: neo.value ?? neo.error }))
    return {
      reference,
      neo: 'value' in neo ? { value: neo.value } : { error: neo.error },
    }
  } finally {
    await runCleanups(cleanups)
  }
}

test('webmcp with the feature on: Neo matches Playwright', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const { reference, neo } = await bothEngines(
    t,
    ['--enable-features=WebMCP'],
    SOURCE,
  )
  assert.deepEqual(reference, { value: EXPECTED }, 'Playwright')
  assert.deepEqual(neo, reference, 'Neo')
})

test('webmcp with the feature off: no tools, in both engines', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const source = `const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
await page.goto(origin + '/tools.html')
return {
  tools: await page.webmcp.tools(),
  call: await page.webmcp.callTool('add', { a: 1, b: 2 }).then(() => 'called', (error) => error.message.split('\\n')[0]),
}`
  const { reference, neo } = await bothEngines(t, [], source)
  assert.deepEqual(
    reference,
    {
      value: {
        tools: [],
        call: 'webMCP.callTool: No WebMCP tool named "add". The frame does not register any WebMCP tools.',
      },
    },
    'Playwright',
  )
  assert.deepEqual(neo, reference, 'Neo')
})
