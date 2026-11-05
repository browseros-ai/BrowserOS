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
const NOT_BUILT = new Set<string>([
  'act',
  'verdict',
  'disarm',
  'frameBox',
  'edit',
  'expect',
])

/** The NOT_BUILT key of one example request. */
const kindOf = (op: string, request: unknown): string => {
  const read = fieldOf(request, 'read')
  if (op === 'read') return `read.${String(fieldOf(read, 'kind'))}`
  if (op === 'readAll') return `readAll.${String(read)}`
  return op
}

/**
 * Request kinds linkedom cannot run: it has no document.open. They still must answer one of
 * their kinds or an error; the live tests cover them in a real browser.
 */
const NEEDS_A_BROWSER = new Set<string>(['setContent'])

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
    resolve(packageRoot, '../../node_modules/.bin/tsc'),
    ['--noEmit', '-p', 'tsconfig.json'],
    { cwd: packageRoot, encoding: 'utf8' },
  )
  expect(`${result.stdout}${result.stderr}`).toBe('')
  expect(result.status).toBe(0)
})

it('tests that every example request gets a reply its op allows from the built probe', () => {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'protocol-test' })
  const wrong: { op: string; reply: unknown; why: string }[] = []
  for (const [op, requests] of Object.entries(REQUEST_EXAMPLES)) {
    for (const request of requests) {
      const reply = page.call(request)
      if (!isValueOp(op)) {
        if (reply !== null && !(reply instanceof Element))
          wrong.push({
            op,
            reply,
            why: 'by-handle replies are an element or null',
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
      if (
        !unbuilt &&
        !NEEDS_A_BROWSER.has(op) &&
        fieldOf(reply, 'class') === 'internal'
      )
        wrong.push({ op, reply, why: 'a built handler failed inside' })
    }
  }
  expect(wrong).toEqual([])
})
