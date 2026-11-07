/**
 * The bundle entry `dist/install.js`: `(key, specSha256, initializer) => glue object`, called
 * by neo-runtime with `Runtime.callFunctionOn {executionContextId}` in Neo's isolated world,
 * once per document, when `attach.js` found nothing under `key`.
 *
 * Builds Playwright's `InjectedScript` with the options Playwright's Chromium server passes
 * (`server/dom.ts:102-112`), wraps the glue table in an object whose methods take
 * `(params, ...handles)` (the shape `call.js` invokes), and stores it on the world's global
 * under `key`. The world belongs to the document and is shared by every CDP session and every
 * Neo run on it, so a concurrent run may have installed first: then this returns that object
 * and builds nothing. That keeps one InjectedScript per document (one hit-target slot, one set
 * of capture listeners) and makes install idempotent. The lookup and the store cannot interleave
 * with another install, because each call runs to completion on the page's one JS thread.
 *
 * `key` is `<SPEC_SHA256>/<initializer JSON>`, built by neo-runtime: a glue object installed by a
 * Neo build with another spec or another test-id attribute is never returned to this one.
 */

import type { InjectedInitializer } from '@browseros/neo-runtime-spec/injected'
import { SPEC_SHA256 } from '@browseros/neo-runtime-spec/spec-hash'
import {
  InjectedScript,
  type InjectedScriptOptions,
} from '../vendor/playwright/injected/src/injectedScript'
import { WebMCPScript } from '../vendor/playwright/injected/src/webMCP'
import {
  parseEvaluationResultValue,
  serializeAsCallArgument,
} from '../vendor/playwright/isomorphic/utilityScriptSerializers'
import { glue, vendored } from './glue'
import { wellFormedCall } from './well-formed'
import { WORLD_KEY } from './world-key'

/** A glue function as `install` binds it, whatever its declared params and handles. */
type GlueFunction = (
  injected: InjectedScript,
  params: never,
  ...handles: never[]
) => unknown

/** The object `call.js` is called on: one method per glue key. */
export type GlueObject = {
  [Name in keyof typeof glue]: (
    params: unknown,
    ...handles: unknown[]
  ) => unknown
}

/**
 * The isolated world's global. A plain null-prototype record, not a `Map`, so glue objects
 * installed by another Neo build in the same document stay readable by this one's `attach`.
 */
export type NeoWorld = typeof globalThis & {
  [WORLD_KEY]?: Record<string, GlueObject>
}

// Neo: the glue's WebMCP calls construct Playwright's `WebMCPScript` (`glue.ts` `vendored`).
vendored.WebMCPScript = WebMCPScript
vendored.parseValue = (value) => parseEvaluationResultValue(value)
// Playwright: `injected/src/utilityScript.ts` `UtilityScript.jsonValue`: `return serializeAsCallArgument(value, (value: any) => ({ fallThrough: value }));`
vendored.serializeValue = (value) =>
  serializeAsCallArgument(value, (inner) => ({ fallThrough: inner }))

export default function install(
  key: string,
  specSha256: string,
  initializer: InjectedInitializer,
): GlueObject {
  // neo-runtime const-asserts dist/spec-sha256 and neo-check rebuilds dist/, so this fires only
  // for a stale bundle that slipped past both.
  if (specSha256 !== SPEC_SHA256) {
    throw new Error(
      `neo-runtime-injected was built from spec ${SPEC_SHA256}, not ${specSha256}`,
    )
  }
  const world = globalThis as NeoWorld
  const store: Record<string, GlueObject> =
    world[WORLD_KEY] ?? Object.create(null)
  world[WORLD_KEY] = store
  const installed = store[key]
  if (installed) return installed

  // Every initializer field must be one of Playwright's options, under the same name.
  const fromSpec: Pick<InjectedScriptOptions, keyof InjectedInitializer> =
    initializer
  const options: InjectedScriptOptions = {
    ...fromSpec,
    isUnderTest: false,
    sdkLanguage: 'javascript',
    browserName: 'chromium',
    shouldPrependErrorPrefix: false,
    customEngines: [],
  }
  const injected = new InjectedScript(window, options)
  // Every glue function accepts this shape: `never` params take any declared params, and the
  // handles CDP passes after them arrive in the declared order. Each answers well-formed
  // (`well-formed.ts`), so no page text can make its CDP reply unreadable.
  const table: Record<string, GlueFunction> = glue
  const object = Object.fromEntries(
    Object.entries(table).map(([name, fn]) => [
      name,
      (params: never, ...handles: never[]) =>
        wellFormedCall(() => fn(injected, params, ...handles)),
    ]),
  ) as GlueObject
  store[key] = object
  return object
}
