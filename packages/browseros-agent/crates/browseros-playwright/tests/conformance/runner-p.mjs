/**
 * Runner P: every conformance case run by real Playwright (playwright-core at the vendored
 * version), connected over CDP to the same throwaway BrowserOS Runner N drives. It is what
 * "what Playwright would give" means for each case.
 *
 * Each case gets a fresh tab from `context.newPage()` and opens its page with `goto`. Every tab
 * the case opened, popups included, is closed after it.
 *
 * A case with a `// @runner-p: stock-chromium` line runs on Playwright's own Chromium
 * (`chromium.launch()`, headless) instead. playwright-core 1.64.1's `setContent` waits for a
 * console message after `document.open()`, which a hosted BrowserOS never sends, so it never
 * sees `load` there; on stock Chromium and on a non-hosted BrowserOS it does (measured, spike
 * S-runner-p). Runner N ignores the line: Neo's setContent needs no console message.
 *
 * It connects as a client co-located with the browser (`isLocal: true`), as Neo is, so
 * `setInputFiles` with paths takes upstream's native path (`DOM.setFileInputFiles`, trusted
 * events) rather than reading the files into payloads. Both runners work in this directory,
 * so a case's relative paths (`files/one.txt`) name the same files for each.
 *
 *   PLAYWRIGHT_NODE_MODULES=<node_modules with playwright-core and @playwright/test 1.64.1> \
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
// Cases call `expect` as a free name, as scripts in Neo's `run` do (claw-sdk hands them the
// prelude's), and harness.js compiles them to see it as a global.
// Playwright Test's own expect is the reference: its matchers poll and fail as upstream's do.
globalThis.expect = require('@playwright/test').expect

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

// CONFORMANCE_CASES_DIR (relative to this directory) picks a tier's cases: tier 2's are in
// cases/tier2, which tier 1's listing skips because it reads only the files directly in cases/.
const casesDir = resolve(here, process.env.CONFORMANCE_CASES_DIR ?? 'cases')
const names = readdirSync(casesDir)
  .filter((file) => file.endsWith('.js'))
  .map((file) => basename(file, '.js'))
  .filter((name) => only.length === 0 || only.includes(name))
  .sort()
const outPath = resolve(outDir)
mkdirSync(outPath, { recursive: true })
process.chdir(here)
/** Whether a case asks to run on Playwright's own Chromium (see the header). */
function onStockChromium(source) {
  return /^\/\/ @runner-p: stock-chromium$/m.test(source)
}

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${cdpPort}`, {
  isLocal: true,
})
/** Launched on the first case that asks for it, closed with the run. */
let stock = null
try {
  const [context] = browser.contexts()
  for (const name of names) {
    const source = readFileSync(resolve(casesDir, `${name}.js`), 'utf8')
    if (onStockChromium(source) && stock === null) {
      const launched = await chromium.launch()
      stock = { browser: launched, context: await launched.newContext() }
    }
    let outcome = await runCase(
      onStockChromium(source) ? stock.context : context,
      source,
    )
    let text
    try {
      text = JSON.stringify(outcome, null, 2)
    } catch (error) {
      outcome = { broke: `the case returned what JSON cannot hold: ${error}` }
      text = JSON.stringify(outcome, null, 2)
    }
    writeFileSync(resolve(outPath, `${name}.json`), `${text}\n`)
    console.log(`runner-p: ${name}: ${Object.keys(outcome)[0]}`)
  }
} finally {
  await stock?.browser.close()
  await browser.close()
}
