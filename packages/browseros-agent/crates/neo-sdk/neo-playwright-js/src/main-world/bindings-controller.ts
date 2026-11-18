/**
 * The page side of `exposeBinding`, as upstream's `PageBinding.createInitScript` installs it: the
 * vendored `BindingsController` at `globalThis.__playwright__binding__controller__`, posting each
 * call to the CDP binding `__playwright__binding__`. The build (scripts/build.ts) bundles it into
 * crates/neo-sdk/neo-playwright/assets/bindings-controller.js, one script that Rust registers in a
 * tab's main world for the run that exposes a binding there (page/init_script.rs), and runs at once
 * in the documents already loaded. A second install, by a second run on the tab, keeps the first.
 *
 * Each binding's own stub is one line Rust adds beside it (`controller.addBinding(name,
 * noGlobal)`); Rust answers a call with `controller.deliverBindingResult` in the context that made
 * it, once the agent's function returned.
 */
import { BindingsController } from '../../../neo-playwright/third_party/playwright/packages/injected/src/bindingsController'
import { kBindingsControllerProperty } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'

/** upstream's `PageBinding.kBindingName`: the CDP binding every call goes through. */
const BINDING_NAME = '__playwright__binding__'

const global = globalThis as Record<string, unknown>
if (!global[kBindingsControllerProperty])
  global[kBindingsControllerProperty] = new BindingsController(
    globalThis,
    BINDING_NAME,
  )
