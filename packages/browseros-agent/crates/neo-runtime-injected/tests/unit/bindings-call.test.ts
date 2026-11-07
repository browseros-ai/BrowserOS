/**
 * The BUILT `dist/bindings-call.js` (what neo-runtime's `bindings.rs` runs in a frame's main
 * world) on a stand-in global, without a browser: the run's `end` leaves a document none of the
 * run's binding globals, whether or not its controller was ever installed.
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { runInThisContext } from 'node:vm'

const source = readFileSync(
  new URL('../../dist/bindings-call.js', import.meta.url),
  'utf8',
)
type BindingsCall = (op: string, arg: unknown) => void
// The bundle is one function expression, as UtilityScript evaluates it, run in this global.
const bindingsCall = runInThisContext(source) as BindingsCall

const CONTROLLER = '__playwright__binding__controller__'
const BINDING = '__playwright__binding__'
const global = globalThis as unknown as Record<string, unknown>
const END = {
  names: [],
  message: 'Target page, context or browser has been closed',
}

afterEach(() => {
  delete global[CONTROLLER]
  delete global[BINDING]
})

describe('bindings-call end', () => {
  test('removes the CDP binding from a document that never got its controller', () => {
    global[BINDING] = () => {}
    bindingsCall('end', END)
    expect(BINDING in global).toBe(false)
  })

  test('rejects each waiting call and removes every binding with the controller', () => {
    const rejected: string[] = []
    const removed: string[] = []
    const callbacks = new Map([
      [1, { reject: (error: Error) => rejected.push(error.message) }],
    ])
    const bindings = new Map([['compute', { callbacks }]])
    global[BINDING] = () => {}
    global[CONTROLLER] = {
      _bindings: bindings,
      removeBinding(name: string) {
        removed.push(name)
        bindings.delete(name)
      },
    }
    bindingsCall('end', END)
    expect(rejected).toEqual([END.message])
    expect(removed).toEqual(['compute'])
    expect(CONTROLLER in global).toBe(false)
    expect(BINDING in global).toBe(false)
  })

  test('another op without a controller changes nothing', () => {
    global[BINDING] = () => {}
    bindingsCall('deliver', { name: 'compute', seq: 1, result: 1 })
    expect(BINDING in global).toBe(true)
  })
})
