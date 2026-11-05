/**
 * Runner P: every conformance case run by real Playwright (playwright-core at the vendored
 * version), connected over CDP to the same throwaway BrowserOS Runner N drives. It is what
 * "what Playwright would give" means for each case.
 *
 * Each case gets a fresh tab from `context.newPage()` and opens its page with `goto`, never
 * `setContent`: playwright-core 1.64.1's `setContent` never sees `load` on a fresh BrowserOS
 * tab (decisions.md). Every tab the case opened, popups included, is closed after it.
 *
 *   PLAYWRIGHT_NODE_MODULES=<node_modules with playwright-core 1.64.1> \
 *   BROWSEROS_CDP_PORT=9521 CONFORMANCE_PORT=<server.mjs port> \
 *   node runner-p.mjs <out dir> [case name ...]
 *
 * It writes `<out dir>/<case>.json` per case and exits 0 unless it could not start.
 */
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { basename, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

const here = dirname(fileURLToPath(import.meta.url))
const CASE_BUDGET_MS = 90_000

function required(name) {
  const value = process.env[name]
  if (!value) {
    console.error(`runner-p: set ${name}`)
    process.exit(2)
  }
  return value
}

const nodeModules = required('PLAYWRIGHT_NODE_MODULES')
const cdpPort = Number(required('BROWSEROS_CDP_PORT'))
const port = Number(required('CONFORMANCE_PORT'))
const [outDir, ...only] = process.argv.slice(2)
if (!outDir) {
  console.error('usage: node runner-p.mjs <out dir> [case name ...]')
  process.exit(2)
}

const require = createRequire(resolve(nodeModules, 'runner-p.cjs'))
const vendored = readFileSync(
  resolve(here, '../../third_party/playwright/VERSION'),
  'utf8',
).split('\n')[0]
const { version } = require('playwright-core/package.json')
if (`v${version}` !== vendored) {
  console.error(
    `runner-p: playwright-core is ${version}; the vendored engine is ${vendored}`,
  )
  process.exit(2)
}
const { chromium } = require('playwright-core')

vm.runInThisContext(readFileSync(resolve(here, 'harness.js'), 'utf8'), {
  filename: 'harness.js',
})
const conformance = globalThis.__conformance

// A promise a case left behind (a waitForEvent it never awaited, or one from a case that ran
// past its budget) may reject later. In Neo's runtime it is dropped with the run; here it
// must not end the process.
process.on('unhandledRejection', (reason) => {
  console.error(`runner-p: a case left a promise that rejected: ${reason}`)
})

/**
 * One CDP session per page for harness.pageEval, kept until the page closes. Detaching a
 * session waits while the page shows a dialog, so a session per call would deadlock a case
 * that clicks a button opening one (measured).
 */
const sessions = new WeakMap()

/** harness.pageEval: the page's main world, with a user gesture, as Neo's runner does it. */
async function pageEval(target, expression) {
  if (!sessions.has(target))
    sessions.set(target, target.context().newCDPSession(target))
  const session = await sessions.get(target)
  const reply = await session.send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
    userGesture: true,
  })
  if (reply.exceptionDetails)
    throw new Error(
      `the page threw: ${reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text}`,
    )
  return reply.result.value ?? null
}

function within(ms, work) {
  let timer
  const expired = new Promise((_, fail) => {
    timer = setTimeout(() => fail(new Error(`the case ran past ${ms} ms`)), ms)
  })
  return Promise.race([work, expired]).finally(() => clearTimeout(timer))
}

async function runCase(context, source) {
  const skip = conformance.needs(source)
  if (skip !== null) return { skipped: skip }
  const before = new Set(context.pages())
  try {
    const page = await context.newPage()
    return await within(
      CASE_BUDGET_MS,
      conformance.run(source, { page, context, port, pageEval }),
    )
  } catch (error) {
    return { broke: String(error?.stack ?? error) }
  } finally {
    for (const opened of context.pages())
      if (!before.has(opened)) await opened.close().catch(() => {})
  }
}

const casesDir = resolve(here, 'cases')
const names = readdirSync(casesDir)
  .filter((file) => file.endsWith('.js'))
  .map((file) => basename(file, '.js'))
  .filter((name) => only.length === 0 || only.includes(name))
  .sort()
mkdirSync(outDir, { recursive: true })
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${cdpPort}`)
try {
  const [context] = browser.contexts()
  for (const name of names) {
    const source = readFileSync(resolve(casesDir, `${name}.js`), 'utf8')
    let outcome = await runCase(context, source)
    let text
    try {
      text = JSON.stringify(outcome, null, 2)
    } catch (error) {
      outcome = { broke: `the case returned what JSON cannot hold: ${error}` }
      text = JSON.stringify(outcome, null, 2)
    }
    writeFileSync(resolve(outDir, `${name}.json`), `${text}\n`)
    console.log(`runner-p: ${name}: ${Object.keys(outcome)[0]}`)
  }
} finally {
  await browser.close()
}
