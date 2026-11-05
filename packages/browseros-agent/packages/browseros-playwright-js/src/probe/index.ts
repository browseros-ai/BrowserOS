/**
 * Neo's probe: the page half of browseros-playwright, and the only code that calls
 * Playwright's InjectedScript. scripts/build.ts bundles it with the vendored engine into
 * crates/browseros-playwright/assets/injected.js: one module graph, so the probe shares the
 * engine's single copy of module state, and one global, `__neoProbe`.
 *
 * Rust installs it once per frame document, in the sealed `browseros-sdk` world:
 *
 *   (() => { <core's seal_source()>;
 *            if (!globalThis.__neoProbe) { <injected.js> }
 *            return globalThis.__neoProbe.boot({ frameSeq, epoch }) })()
 *
 * The seal comes first in the same script, so a seal that throws leaves no engine behind,
 * and `boot` refuses a world without one. After that every request is one CALL_FUNCTION
 * (protocol.ts). Each request family has its own handler file; this one owns the lifecycle
 * (boot and re-key), the dispatch table, the error replies, and the gate that answers
 * `needsMask` for a reply whose page text the engine cut without the call's mask
 * (page-mask.ts).
 */
import { isInvalidSelectorError } from '../../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { RefMap } from './aria-ref'
import { documentText, replaceDocument } from './document'
import { underMask } from './page-mask'
import {
  type BootOptions,
  type Described,
  type ErrorClass,
  type ErrorReply,
  type Mask,
  type NeedsMask,
  type NeoProbe,
  PROTOCOL,
  type ProbeRequest,
  type ReplyTo,
  type RequestOf,
  type Uninstalled,
  type ValueOp,
} from './protocol'
import { read, readAll } from './read'
import { pinned, resolve } from './resolve'
import { focused, sealIn, sweep } from './sensitive'
import { snapshot } from './snapshot'
import { matchUrl } from './url'
import {
  createEngine,
  engineTable,
  notBuilt,
  Pins,
  ProbeError,
  type World,
} from './world'

/**
 * The requests the table routes. `setContent` is not one: it changes the probe's own
 * lifecycle, so `call` handles it, as `boot` handles booting.
 */
type Routed = Exclude<ValueOp, 'setContent'>
type Handler<O extends Routed> = (
  world: World,
  request: RequestOf<O>,
  root: Node | undefined,
) => ReplyTo<O>
type Handlers = { [O in Routed]: Handler<O> }

/** One entry per request kind: adding a kind is a line here plus its handler file. */
const HANDLERS: Handlers = {
  describe: described,
  resolve,
  act: notBuilt,
  verdict: notBuilt,
  disarm: notBuilt,
  frameBox: notBuilt,
  edit: notBuilt,
  read,
  readAll,
  snapshot,
  expect: notBuilt,
  sweep,
  focused,
  matchUrl,
  document: documentText,
}

const UNINSTALLED: Uninstalled = { kind: 'uninstalled' }
const NEEDS_MASK: NeedsMask = { kind: 'needsMask' }

function described(world: World): Described {
  return {
    kind: 'described',
    protocol: PROTOCOL,
    seq: world.seq,
    url: location.href,
  }
}

function createProbe(): NeoProbe {
  // One table and one map per document, across re-keys: their numbers never restart, so a
  // pin or frame key handed out under another epoch can only fail here, never alias.
  const pins = new Pins()
  const refs = new RefMap()
  let world: World | undefined
  // The seq of the last boot, kept when setContent unboots: whoever booted it may still hold
  // refs printed under it, so the next boot hands it to Rust to retire.
  let lastSeq: number | undefined

  /** One request's reply in `booted`, the error replies included. */
  function answer(
    booted: World,
    request: ProbeRequest,
    scope: Node | undefined,
  ): unknown {
    try {
      if (request.op === 'pinned') return pinned(booted, request)
      // document.open() keeps the window, and with it this world and this probe. From the
      // moment it returns, the engine, the seq, every pin and every ref belong to the old
      // document, so the probe unboots: the next request answers `uninstalled`, and Rust
      // boots the new document under a new seq, as for any other new document.
      if (request.op === 'setContent')
        return replaceDocument(booted, request, () => {
          world = undefined
        })
      return dispatch(booted, request.op, request, scope)
    } catch (error) {
      return errorReply(
        classify(error),
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  return {
    boot(options) {
      const { frameSeq, epoch } = parseBootOptions(options)
      if (world?.epoch === epoch) return described(world)
      const seal = sealIn(globalThis)
      if (!seal)
        throw new ProbeError(
          'internal',
          'refusing to boot: this world has no seal (__clawSeal)',
        )
      // Another epoch is another claw-sdk process, or this one after a restart. Nothing it
      // was handed may resolve under the new seq: pins, refs and the engine's ref expandos
      // are dropped, and a fresh engine takes the new seq.
      pins.clear()
      refs.forget()
      const injected = createEngine(frameSeq)
      refs.install(engineTable(injected), injected.utils)
      world = { epoch, seq: frameSeq, injected, seal, pins, refs }
      const retired = lastSeq
      lastSeq = frameSeq
      return retired === undefined
        ? described(world)
        : { ...described(world), retired }
    },

    call(request, root, mask) {
      const booted = world
      if (!booted) return UNINSTALLED
      if (!hasKnownOp(request))
        return errorReply('internal', 'not a probe request')
      if (!isMask(mask)) return errorReply('internal', 'not a probe mask')
      // On a context call CALL_FUNCTION passes the world's global as `root`.
      const scope = root instanceof Node ? root : undefined
      // Rust's mask is the site as its sweep found it, a round trip or more ago, and this
      // frame's fields may have been typed into since. Read now, in the same synchronous call
      // as every cut, this frame's own values are never newer than the mask; Rust's sweep
      // after the call catches another frame of the site that changed (resolve::masked_call).
      const callMask =
        mask === undefined
          ? undefined
          : [...mask, ...booted.seal.sensitiveValues()]
      // A reply that holds page text the engine cut or quoted without a mask never leaves
      // the page: Rust is asked for the mask instead, and sends the request again with it.
      const { value, needsMask } = underMask(callMask, () =>
        answer(booted, request, scope),
      )
      return needsMask ? NEEDS_MASK : value
    },
  }
}

/** No mask, or Rust's: an array of strings. */
function isMask(mask: unknown): mask is Mask | undefined {
  return (
    mask === undefined ||
    (Array.isArray(mask) && mask.every((value) => typeof value === 'string'))
  )
}

/** `op` travels beside the request so the table lookup and the request agree on one op. */
function dispatch<O extends Routed>(
  world: World,
  op: O,
  request: RequestOf<O>,
  root: Node | undefined,
): ReplyTo<O> {
  const handler: Handler<O> = HANDLERS[op]
  return handler(world, request, root)
}

/**
 * Only the op is checked. Rust and this bundle are version-locked by PROTOCOL and decode the
 * same probe-protocol.json examples, so a request with a known op carries that op's fields;
 * a malformed one fails inside its handler as `internal`.
 */
function hasKnownOp(request: unknown): request is ProbeRequest {
  if (typeof request !== 'object' || request === null) return false
  const op: unknown = Reflect.get(request, 'op')
  return (
    typeof op === 'string' &&
    (op === 'pinned' || op === 'setContent' || Object.hasOwn(HANDLERS, op))
  )
}

function parseBootOptions(options: unknown): BootOptions {
  const frameSeq: unknown =
    typeof options === 'object' && options !== null
      ? Reflect.get(options, 'frameSeq')
      : undefined
  const epoch: unknown =
    typeof options === 'object' && options !== null
      ? Reflect.get(options, 'epoch')
      : undefined
  if (
    typeof frameSeq !== 'number' ||
    !Number.isSafeInteger(frameSeq) ||
    frameSeq < 0 ||
    typeof epoch !== 'string' ||
    epoch === ''
  )
    throw new ProbeError(
      'internal',
      'boot takes { frameSeq: a non-negative integer, epoch: a non-empty string }',
    )
  return { frameSeq, epoch }
}

/**
 * Probe code throws only ProbeErrors, each with its class. Playwright's engine throws plain
 * Errors (createStacklessError) for facts about the page the script has to handle, such as
 * "Not a checkbox or radio button": those are `script`. Selector syntax errors are
 * `selector`. Anything else, such as a TypeError from a probe bug, is `internal`.
 */
function classify(error: unknown): ErrorClass {
  if (error instanceof ProbeError) return error.errorClass
  if (error instanceof Error && isInvalidSelectorError(error)) return 'selector'
  if (
    error instanceof Error &&
    Object.getPrototypeOf(error) === Error.prototype
  )
    return 'script'
  return 'internal'
}

function errorReply(errorClass: ErrorClass, message: string): ErrorReply {
  return { kind: 'error', message, class: errorClass }
}

// The install expression skips this bundle when the global exists; `??=` keeps a second
// evaluation harmless too, because the first probe and its engine stay in charge.
globalThis.__neoProbe ??= createProbe()

declare global {
  var __neoProbe: NeoProbe | undefined
}
