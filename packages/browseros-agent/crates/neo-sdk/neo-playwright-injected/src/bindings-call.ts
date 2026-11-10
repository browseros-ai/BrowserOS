/**
 * Playwright: `server/page.ts` `PageBinding.constructor`
 * Playwright: `server/page.ts` `PageBinding.dispatch`
 *
 * The bundle entry `dist/bindings-call.js`: `(op, arg) => void`, every call neo-runtime's
 * `bindings.rs` makes on the page's `BindingsController` once `dist/bindings-controller.js`
 * installed it, chosen by name so that no page code travels from Rust. It runs as an init
 * script (`add`) and as `UtilityScript.evaluate`'s expression in a frame's main world (every op),
 * where `deliver`'s result may hold the run's handles.
 */

import type { BindingsController } from '../vendor/playwright/injected/src/bindingsController'

/** Playwright: `isomorphic/utilityScriptSerializers.ts` `kBindingsControllerProperty`: `export const kBindingsControllerProperty = '__playwright__binding__controller__';` */
const CONTROLLER = '__playwright__binding__controller__'

/** Playwright: `server/page.ts` `PageBinding.kBindingName`: `static kBindingName = '__playwright__binding__';` */
const BINDING = '__playwright__binding__'

type Delivery = Parameters<BindingsController['deliverBindingResult']>[0]

/** Neo: the controller's own state that `end` reads: its bindings and their waiting calls. */
type Internals = {
  _bindings: Map<
    string,
    { callbacks: Map<number, { reject: (error: Error) => void }> }
  >
}

type Ops = {
  add: { name: string; noGlobal: boolean }
  remove: { name: string }
  deliver: Delivery
  end: { names: string[]; message: string }
}

/** Neo: runs `op` on the page's controller. A document without one has nothing to do, but at
 * the run's end: a setup cut short between `Runtime.addBinding` and the controller's install
 * left it the CDP binding function, which goes as it goes from any other document. */
export default function bindingsCall<Op extends keyof Ops>(
  op: Op,
  arg: Ops[Op],
): void {
  const global = globalThis as unknown as Record<string, unknown>
  const controller = global[CONTROLLER] as BindingsController | undefined
  if (!controller) {
    if (op === 'end') delete global[BINDING]
    return
  }
  switch (op) {
    case 'add': {
      const { name, noGlobal } = arg as Ops['add']
      // Playwright: `server/page.ts` `PageBinding.constructor`: `.addBinding(${JSON.stringify(name)}, ${!!noGlobal})`
      controller.addBinding(name, noGlobal)
      return
    }
    case 'remove':
      // Playwright: `server/page.ts` `PageBinding.constructor`: `.removeBinding(${JSON.stringify(name)})`
      controller.removeBinding((arg as Ops['remove']).name)
      return
    case 'deliver':
      // Playwright: `server/page.ts` `PageBinding.dispatch`: `.deliverBindingResult(arg)`
      controller.deliverBindingResult(arg as Delivery)
      return
    case 'end':
      endRun(controller, arg as Ops['end'], global)
      return
  }
}

/**
 * Deviation: `server/page.ts` `Page.removeExposedBinding`: `await this.delegate.removeInitScripts([binding.initScript]);` Playwright's bindings live as long as their page; Neo's end with the run, and a call still waiting on one is refused.
 *
 * Every call still waiting on one of the run's bindings (`names`, every name the run began to
 * expose) rejects with `message`; each of them is removed, as Playwright's cleanup script
 * removes it. Another run's names on the same page stay, with the controller they need; the
 * controller goes once it holds none, and the CDP binding function, which outlives the session
 * that added it (probe C2), so the page keeps nothing of the runs.
 */
function endRun(
  controller: BindingsController,
  { names, message }: Ops['end'],
  global: Record<string, unknown>,
): void {
  const bindings = (controller as unknown as Internals)._bindings
  if (typeof bindings?.get !== 'function')
    throw new Error(
      'neo-playwright-injected: BindingsController has no _bindings',
    )
  for (const name of new Set(names)) {
    for (const waiting of bindings.get(name)?.callbacks.values() ?? [])
      waiting.reject(new Error(message))
    controller.removeBinding(name)
  }
  if (bindings.size === 0) {
    delete global[CONTROLLER]
    delete global[BINDING]
  }
}
