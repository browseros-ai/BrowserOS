/**
 * Neo's probe: the page half of neo-playwright, and the only code that calls
 * Playwright's InjectedScript. scripts/build.ts bundles it with the vendored engine into
 * crates/neo-sdk/neo-playwright/assets/injected.js: one module graph, so the probe shares the
 * engine's single copy of module state, and one global, `__neoProbe`.
 *
 * Rust installs it once per frame document, in the isolated `browseros-engine` world:
 *
 *   (() => { if (!globalThis.__neoProbe) { <injected.js> }
 *            return globalThis.__neoProbe.boot({ frameSeq, epoch }) })()
 *
 * After that every request is one CALL_FUNCTION (protocol.ts). Each request family has its own handler file; this one owns the lifecycle
 * (boot and re-key), the dispatch table and the error replies.
 */
import { isInvalidSelectorError } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { act, disarm, frameBox, listenForMoves, moves, verdict } from './act'
import { RefMap } from './aria-ref'
import { hideBrowserOSUi } from './browseros-ui'
import { documentText, replaceDocument } from './document'
import { edit } from './edit'
import { expect } from './expect'
import { armInput, inputArrived } from './files'
import {
  type BootOptions,
  type Described,
  type ErrorClass,
  type ErrorReply,
  PROTOCOL,
  type ProbeRequest,
  type ReplyTo,
  type RequestOf,
  type Resolution,
  type Uninstalled,
  type ValueOp,
} from './protocol'
import { read, readAll } from './read'
import { generatedSelector, pinned, resolve } from './resolve'
import { screenshotMask, screenshotMaskCleanup } from './screenshot'
import { snapshot } from './snapshot'
import { matchUrl } from './url'
import { webStorage } from './web-storage'
import {
  createEngine,
  engineTable,
  Pins,
  ProbeError,
  type World,
} from './world'

/**
 * The requests the table routes: every request answered by value. A handler that must await
 * the page (`expect`'s viewport check) answers with a promise.
 */
type Routed = ValueOp
type Handler<O extends Routed> = (
  world: World,
  request: RequestOf<O>,
  root: Node | undefined,
  nodes: readonly unknown[],
) => ReplyTo<O> | Promise<ReplyTo<O>>
type Handlers = { [O in Routed]: Handler<O> }

/** One entry per request kind: adding a kind is a line here plus its handler file. */
const HANDLERS: Handlers = {
  describe: described,
  screenshotMask,
  screenshotMaskCleanup,
  resolve,
  act,
  verdict,
  moves,
  disarm,
  frameBox,
  edit,
  read,
  readAll,
  selector: generatedSelector,
  snapshot,
  expect,
  matchUrl,
  document: documentText,
  setContent: replaceDocument,
  armInput,
  inputArrived,
  webStorage,
}

const UNINSTALLED: Uninstalled = { kind: 'uninstalled' }

function described(world: World): Described {
  return {
    kind: 'described',
    protocol: PROTOCOL,
    seq: world.seq,
    url: location.href,
  }
}

function createProbe() {
  // One table and one map per document, across re-keys: their numbers never restart, so a
  // pin or frame key handed out under another epoch can only fail here, never alias.
  const pins = new Pins()
  const refs = new RefMap()
  let world: World | undefined

  /** One request's reply in `booted`, the error replies included. */
  function answer(
    booted: World,
    request: ProbeRequest,
    scope: Node | undefined,
    nodes: readonly unknown[],
  ): unknown {
    try {
      if (request.op === 'pinned') return pinned(booted, request)
      const reply = dispatch(booted, request.op, request, scope, nodes)
      // CDP's `awaitPromise` settles a promise for Rust, so a handler that throws after
      // its first await is classified here, as a synchronous one is below.
      return reply instanceof Promise ? reply.catch(thrownReply) : reply
    } catch (error) {
      return thrownReply(error)
    }
  }

  return {
    boot(options: BootOptions) {
      const { frameSeq, epoch } = parseBootOptions(options)
      if (world?.epoch === epoch) return described(world)
      // Another epoch is another claw-sdk process, or this one after a restart. Nothing it
      // was handed may resolve under the new seq: pins, refs and the engine's ref expandos
      // are dropped, and a fresh engine takes the new seq.
      pins.clear()
      refs.forget()
      listenForMoves()
      hideBrowserOSUi()
      const injected = createEngine(frameSeq)
      injected.onGlobalListenersRemoved.add(listenForMoves)
      refs.install(engineTable(injected), injected.utils)
      const retired = world?.seq
      world = { epoch, seq: frameSeq, injected, pins, refs }
      return retired === undefined
        ? described(world)
        : { ...described(world), retired }
    },

    call(request: unknown, root?: unknown, nodes?: unknown) {
      const booted = world
      if (!booted) return UNINSTALLED
      if (!hasKnownOp(request))
        return errorReply('internal', 'not a probe request')
      // On a context call CALL_FUNCTION passes the world's global as `root`.
      const scope = root instanceof Node ? root : undefined
      // The nodes after the root, in Rust's order; each handler checks what it reads.
      const handed: readonly unknown[] = Array.isArray(nodes) ? nodes : []
      return answer(booted, request, scope, handed)
    },
  }
}

/** `op` travels beside the request so the table lookup and the request agree on one op. */
function dispatch<O extends Routed>(
  world: World,
  op: O,
  request: RequestOf<O>,
  root: Node | undefined,
  nodes: readonly unknown[],
): ReplyTo<O> | Promise<ReplyTo<O>> {
  const handler: Handler<O> = HANDLERS[op]
  return handler(world, request, root, nodes)
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
    typeof op === 'string' && (op === 'pinned' || Object.hasOwn(HANDLERS, op))
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

function thrownReply(error: unknown): ErrorReply {
  const reply = errorReply(
    classify(error),
    error instanceof Error ? error.message : String(error),
  )
  // A file-input read's refusal carries what its chunk resolved to (read.ts).
  const resolved: unknown =
    error instanceof Error ? Reflect.get(error, 'resolved') : undefined
  return isResolution(resolved) ? { ...reply, resolved } : reply
}

function isResolution(value: unknown): value is Resolution {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof Reflect.get(value, 'preview') === 'string' &&
    typeof Reflect.get(value, 'count') === 'number'
  )
}

function errorReply(errorClass: ErrorClass, message: string): ErrorReply {
  return { kind: 'error', message, class: errorClass }
}

// The install expression skips this bundle when the global exists; `??=` keeps a second
// evaluation harmless too, because the first probe and its engine stay in charge.
globalThis.__neoProbe ??= createProbe()

/** The probe ABI follows its actual implementation; its data comes from generated types. */
type NeoProbe = ReturnType<typeof createProbe>

declare global {
  var __neoProbe: NeoProbe | undefined
}
