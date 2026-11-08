/**
 * The BUILT `dist/bindings-call.js` (what neo-runtime's `bindings.rs` runs in a frame's main
 * world) on a stand-in global, without a browser: the run's `end` leaves a document none of the
 * run's binding globals, whether or not its controller was ever installed, and leaves another
 * run's names with the controller and the CDP binding they need.
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

  /** A stand-in controller holding `names`, each with one waiting call. */
  function controllerOf(names: string[]) {
    const rejected: string[] = []
    const removed: string[] = []
    const bindings = new Map(
      names.map((name) => [
        name,
        {
          callbacks: new Map([
            [
              1,
              {
                reject: (error: Error) =>
                  rejected.push(`${name}:${error.message}`),
              },
            ],
          ]),
        },
      ]),
    )
    global[BINDING] = () => {}
    global[CONTROLLER] = {
      _bindings: bindings,
      removeBinding(name: string) {
        removed.push(name)
        bindings.delete(name)
      },
    }
    return { rejected, removed }
  }

  test('rejects the run’s waiting calls and removes its bindings with the controller', () => {
    const { rejected, removed } = controllerOf(['compute'])
    bindingsCall('end', { ...END, names: ['compute'] })
    expect(rejected).toEqual([`compute:${END.message}`])
    expect(removed).toEqual(['compute'])
    expect(CONTROLLER in global).toBe(false)
    expect(BINDING in global).toBe(false)
  })

  test('leaves another run’s names with the controller and the CDP binding', () => {
    const { rejected, removed } = controllerOf(['runA', 'runB'])
    bindingsCall('end', { ...END, names: ['runA'] })
    expect(rejected).toEqual([`runA:${END.message}`])
    expect(removed).toEqual(['runA'])
    expect(CONTROLLER in global).toBe(true)
    expect(BINDING in global).toBe(true)
  })

  test('another op without a controller changes nothing', () => {
    global[BINDING] = () => {}
    bindingsCall('deliver', { name: 'compute', seq: 1, result: 1 })
    expect(BINDING in global).toBe(true)
  })
})
