// The probe protocol from the TypeScript side: the typed examples (which the Rust mirror
// decodes from probe-protocol.json, kept fresh by build.test.ts) type-check against
// protocol.ts, and every example request gets an answer its op allows from the built asset.
import { afterEach, expect, it } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { REPLY_KINDS, type ValueOp } from '../src/probe/protocol'
import { REQUEST_EXAMPLES } from '../src/probe/protocol-examples'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body>
  <button>Go</button><button>Stop</button>
  <input id="pw" type="password" value="hunter2">
  <iframe id="pay" src="https://pay.example/frame"></iframe>
  <ul><li>One</li><li>Two</li></ul>
  <main><h1 id="greeting">Welcome</h1></main>
</body></html>`

/**
 * Request kinds later units implement, by op, or by op and read for the reads (`read.state`).
 * Each unit deletes its line as it builds the kind.
 */
const NOT_BUILT = new Set<string>([])

/** The NOT_BUILT key of one example request. */
const kindOf = (op: string, request: unknown): string => {
  const read = fieldOf(request, 'read')
  if (op === 'read') return `read.${String(fieldOf(read, 'kind'))}`
  if (op === 'readAll') return `readAll.${String(read)}`
  if (op === 'edit')
    return `edit.${String(fieldOf(fieldOf(request, 'edit'), 'kind'))}`
  return op
}

/**
 * Request kinds linkedom cannot run: it has no document.open, and no localStorage or
 * sessionStorage. They still must answer one of their kinds or an error; the live tests and
 * the spec run cover them in a real browser.
 */
const NEEDS_A_BROWSER = new Set<string>([
  'setContent',
  'screenshotMask',
  'webStorage',
])

/**
 * Built handlers whose example answers `internal` on this page by design: a fresh page has
 * armed no interceptor, and a verdict for an unarmed token must not vouch for input nobody
 * checked; linkedom computes no styles, which a frame's box is read from.
 * tests/actions_live.rs runs them on a real layout.
 */
const INTERNAL_ON_A_FRESH_PAGE = new Set<string>(['verdict', 'frameBox'])

let page: ProbePage | undefined
afterEach(() => page?.close())

const isValueOp = (op: string): op is ValueOp => Object.hasOwn(REPLY_KINDS, op)

const fieldOf = (reply: unknown, name: string): unknown =>
  typeof reply === 'object' && reply !== null
    ? Reflect.get(reply, name)
    : undefined

it('tests that the protocol examples type-check against protocol.ts', () => {
  const packageRoot = resolve(import.meta.dir, '..')
  const result = spawnSync(
    resolve(packageRoot, '../../../node_modules/.bin/tsc'),
    ['--noEmit', '-p', 'tsconfig.json'],
    { cwd: packageRoot, encoding: 'utf8' },
  )
  expect(`${result.stdout}${result.stderr}`).toBe('')
  expect(result.status).toBe(0)
})

/**
 * Whether a by-handle reply has its shape: `pinned` answers its element or null, `privateHosts`
 * the camera's waiting hosts, a list of elements Rust reads through deep serialization.
 */
const handedOver = (op: string, reply: unknown): boolean =>
  op === 'privateHosts'
    ? Array.isArray(reply) && reply.every((host) => host instanceof Element)
    : reply === null || reply instanceof Element

it('tests that every example request gets a reply its op allows from the built probe', async () => {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'protocol-test' })
  const wrong: { op: string; reply: unknown; why: string }[] = []
  for (const [op, requests] of Object.entries(REQUEST_EXAMPLES)) {
    for (const request of requests) {
      // `expect` answers with a promise, which CDP's `awaitPromise` settles for Rust.
      const reply = await page.call(request)
      if (!isValueOp(op)) {
        if (!handedOver(op, reply))
          wrong.push({
            op,
            reply,
            why: 'by-handle replies are an element or null, or a list of elements',
          })
        continue
      }
      const kind = fieldOf(reply, 'kind')
      const allowed: readonly unknown[] = [...REPLY_KINDS[op], 'error']
      if (!allowed.includes(kind))
        wrong.push({
          op,
          reply,
          why: `kind is not one of ${allowed.join(', ')}`,
        })
      const notBuilt =
        kind === 'error' && fieldOf(reply, 'message') === 'not built yet'
      const unbuilt = NOT_BUILT.has(kindOf(op, request))
      if (unbuilt !== notBuilt)
        wrong.push({
          op,
          reply,
          why: unbuilt ? 'should still be unbuilt' : 'is routed nowhere',
        })
      // linkedom lacks Element.getHTML; the upstream shadow-content spec exercises that
      // browser serializer. Keep ordinary document reads covered by this offline matrix.
      const needsBrowser =
        NEEDS_A_BROWSER.has(op) ||
        (request.op === 'document' && request.includeShadow === true)
      if (
        !unbuilt &&
        !needsBrowser &&
        fieldOf(reply, 'class') === 'internal' &&
        !INTERNAL_ON_A_FRESH_PAGE.has(op)
      )
        wrong.push({ op, reply, why: 'a built handler failed inside' })
    }
  }
  expect(wrong).toEqual([])
})
