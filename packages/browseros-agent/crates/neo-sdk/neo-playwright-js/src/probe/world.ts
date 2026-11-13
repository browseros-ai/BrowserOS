/**
 * What one booted world owns and every handler shares: Playwright's engine for this
 * document, the pin table and Neo's ref map.
 *
 * This is also the one file that reaches into InjectedScript's private fields. Each access
 * checks the shape it expects and fails as `internal` otherwise, so an upgrade that renames
 * a field breaks loudly at the first request instead of resolving refs or emitting diagnostics
 * wrongly. Request-owned formatting scopes and arm-owned receivers retain the same engine.
 */
import {
  type FrameExpectParams,
  InjectedScript,
} from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import type { RefMap } from './aria-ref'
import type { ErrorClass, Pin } from './protocol'

/** The state a boot builds. A boot with another epoch builds a new one (see index.ts). */
export type World = {
  /** The `Playwright` instance (one claw-sdk process) this world answers to. */
  readonly epoch: string
  /** This document's frame seq; ai-mode refs print as `f<seq>e<n>`, or `e<n>` for seq 0. */
  readonly seq: number
  readonly injected: InjectedScript
  readonly pins: Pins
  readonly refs: RefMap
}

/**
 * A failure the probe classifies itself. Probe code throws only these, because index.ts reads
 * a plain Error as one of Playwright's own (`script`).
 */
export class ProbeError extends Error {
  constructor(
    readonly errorClass: ErrorClass,
    message: string,
  ) {
    super(message)
  }
}

/**
 * Past this many entries the oldest go first. An op needs a handful; this is a leak bound. A
 * batch (`addAll`) is one entry, so `$$` over any number of matches cannot evict its own pins
 * before Rust has made their handles.
 */
const PIN_LIMIT = 512

/**
 * Elements Rust refers to across the calls of one op, by number. Playwright's inner retry
 * loop acts on the element it waited for, and a pin is that element. An evicted or
 * disconnected pin reads as detached, which sends the op back to re-resolve, so eviction
 * heals itself. Numbers never repeat within a document, so an old pin can only fail.
 */
export class Pins {
  private readonly elements = new Map<Pin, Element | WeakRef<Element>>()
  /** The pins of each entry, oldest first: one for `add`, all of a batch for `addAll`. */
  private readonly entries = new Map<number, Pin[]>()
  private next = 1
  private nextEntry = 1

  add(element: Element): Pin {
    const [pin = 0] = this.addAll([element])
    return pin
  }

  /**
   * A frame hop needs its iframe only while connected. Keep that reference weak so the
   * parent's long-lived probe cannot retain an entire removed child document after an op.
   * Element pins stay strong until Rust has converted them to CDP handles.
   */
  addFrame(element: Element): Pin {
    const [pin = 0] = this.record([new WeakRef(element)])
    return pin
  }

  /** Pins every element, in order, as one entry toward the limit. */
  addAll(elements: Element[]): Pin[] {
    return this.record(elements)
  }

  private record(elements: (Element | WeakRef<Element>)[]): Pin[] {
    const pins = elements.map((element) => {
      const pin = this.next++
      this.elements.set(pin, element)
      return pin
    })
    this.entries.set(this.nextEntry++, pins)
    while (this.entries.size > PIN_LIMIT) {
      const oldest = this.entries.entries().next()
      if (oldest.done) break
      this.entries.delete(oldest.value[0])
      for (const pin of oldest.value[1]) this.elements.delete(pin)
    }
    return pins
  }

  /** The pinned element, while it is still in the document. */
  live(pin: Pin): Element | undefined {
    const element = this.held(pin)
    return element?.isConnected ? element : undefined
  }

  /**
   * The pinned element, in the document or not, until its pin is evicted: what Rust makes a
   * handle of. A match that left the page after it was pinned still gets its handle, which
   * then reads as detached, as upstream's handle to that element would. Frame-hop pins may
   * already be collected after removal; they are used only to enter a connected iframe.
   */
  held(pin: Pin): Element | undefined {
    const entry = this.elements.get(pin)
    return entry instanceof WeakRef ? entry.deref() : entry
  }

  /** Forgets every pin (a re-key). Numbering continues. */
  clear(): void {
    this.elements.clear()
    this.entries.clear()
  }
}

/** Playwright's own construction for Chromium (server/dom.ts), in Neo's isolated world. */
export function createEngine(frameSeq: number): InjectedScript {
  return new InjectedScript(window, {
    isUnderTest: false,
    sdkLanguage: 'javascript',
    frameSeq,
    testIdAttributeName: 'data-testid',
    stableRafCount: 1,
    browserName: 'chromium',
    shouldPrependErrorPrefix: false,
    isUtilityWorld: true,
    // Neo's aria-ref engine is set into the table after construction (aria-ref.ts says why).
    customEngines: [],
  })
}

/** The part of a selector engine InjectedScript calls. */
export type QueryEngine = { queryAll(root: Node, body: unknown): Element[] }

/** InjectedScript's engine table (private upstream), keyed by engine name. */
export function engineTable(
  injected: InjectedScript,
): Map<string, QueryEngine> {
  const table: unknown = Reflect.get(injected, '_engines')
  if (!(table instanceof Map))
    throw new ProbeError(
      'internal',
      'the vendored InjectedScript has no _engines table',
    )
  return table
}

/**
 * The refs the engine's last `ariaSnapshotJSON` minted, each with its element (private
 * `_lastAriaSnapshotForQuery.info` upstream, which the built-in aria-ref engine reads).
 */
export function lastSnapshotRefs(
  injected: InjectedScript,
): Map<string, { element: Element }> {
  const snapshot: unknown = Reflect.get(injected, '_lastAriaSnapshotForQuery')
  const info: unknown =
    typeof snapshot === 'object' && snapshot !== null
      ? Reflect.get(snapshot, 'info')
      : undefined
  if (!(info instanceof Map))
    throw new ProbeError(
      'internal',
      'the vendored InjectedScript keeps no _lastAriaSnapshotForQuery.info',
    )
  return info
}

/** What one of the engine's expect checks found: its own result, before `.not`. */
export type ExpectCheck = { matches: boolean; received?: unknown }

/** InjectedScript's per-element and per-array expect checks (private upstream). */
export type ExpectChecks = {
  single(
    element: Element,
    params: FrameExpectParams,
    frameVisible: boolean,
  ): Promise<ExpectCheck>
  every(elements: Element[], params: FrameExpectParams): ExpectCheck
}

/**
 * `expectSingleElement` and `expectArray`, the checks behind the engine's public `expect`.
 * The public one also renders an aria snapshot on every failing poll, for the test runner's
 * error context, which Neo has no place for; these two are what it calls to match.
 * In the pinned vendor, aria/name/description/text branches finish their reads synchronously
 * before the Promise returns; only viewport awaits. FailureOutput relies on that split to
 * restore tree guards before awaiting. Recheck those branches at each vendor upgrade.
 */
export function expectChecks(injected: InjectedScript): ExpectChecks {
  const single: unknown = Reflect.get(injected, 'expectSingleElement')
  const every: unknown = Reflect.get(injected, 'expectArray')
  if (typeof single !== 'function' || typeof every !== 'function')
    throw new ProbeError(
      'internal',
      'the vendored InjectedScript has no expectSingleElement or expectArray',
    )
  return {
    single: (element, params, frameVisible) =>
      Reflect.apply(single, injected, [element, params, frameVisible]),
    every: (elements, params) =>
      Reflect.apply(every, injected, [elements, params]),
  }
}

/** The two vendor diagnostic seams; neither changes normal selector execution. */
type DiagnosticHooks = {
  previewNode: InjectedScript['previewNode']
  generateSelectors(elements: Element[], generate: () => string[]): string[]
}

/**
 * A checked view of the pinned vendor's diagnostic methods. It captures native formatting
 * once, scopes synchronous hooks, and gives delayed hit tests a receiver owned by their arm.
 */
export type DiagnosticEngine = {
  previewNode: InjectedScript['previewNode']
  run<T>(hooks: DiagnosticHooks, invoke: () => T): T
  arm(
    previewNode: InjectedScript['previewNode'],
    ...args: Parameters<InjectedScript['setupHitTargetInterceptor']>
  ): ReturnType<InjectedScript['setupHitTargetInterceptor']>
}

type DiagnosticOverride = {
  property: string
  original: PropertyDescriptor | undefined
  replacement: PropertyDescriptor
}

const diagnosticEngines = new WeakMap<InjectedScript, DiagnosticEngine>()

function diagnosticOverride(
  injected: InjectedScript,
  property: string,
  value: DiagnosticHooks['previewNode'] | ((elements: Element[]) => string[]),
): DiagnosticOverride {
  const original = Object.getOwnPropertyDescriptor(injected, property)
  if (
    original?.configurable === false ||
    (!original && !Object.isExtensible(injected))
  )
    throw new ProbeError(
      'internal',
      'vendor diagnostic hooks cannot be installed',
    )
  return {
    property,
    original,
    replacement: {
      configurable: true,
      enumerable: original?.enumerable ?? false,
      writable: true,
      value,
    },
  }
}

/** Restoration attempts both seams even after one fails, and refuses any incomplete cleanup. */
function restoreDiagnostics(
  injected: InjectedScript,
  installed: DiagnosticOverride[],
): void {
  let failed = false
  for (const override of installed.reverse()) {
    try {
      if (override.original)
        Object.defineProperty(injected, override.property, override.original)
      else if (!Reflect.deleteProperty(injected, override.property))
        failed = true
    } catch {
      failed = true
    }
  }
  if (failed)
    throw new ProbeError(
      'internal',
      'vendor diagnostic hooks could not be restored',
    )
}

export function diagnosticEngine(injected: InjectedScript): DiagnosticEngine {
  const captured = diagnosticEngines.get(injected)
  if (captured) return captured
  const previewNode = injected.previewNode
  const setup = injected.setupHitTargetInterceptor
  const selectors: unknown = Reflect.get(injected, '_generateSelectors')
  const interceptor: unknown = Reflect.get(injected, '_hitTargetInterceptor')
  const interceptorDescriptor = Object.getOwnPropertyDescriptor(
    injected,
    '_hitTargetInterceptor',
  )
  if (
    typeof previewNode !== 'function' ||
    typeof setup !== 'function' ||
    typeof selectors !== 'function' ||
    !interceptorDescriptor ||
    !('value' in interceptorDescriptor) ||
    !interceptorDescriptor.writable ||
    (interceptor !== undefined && typeof interceptor !== 'function')
  )
    throw new ProbeError(
      'internal',
      'the vendored diagnostic engine contract changed',
    )

  const generateSelectors = (elements: Element[]): string[] => {
    const lines: unknown = Reflect.apply(selectors, injected, [elements])
    if (
      !Array.isArray(lines) ||
      !lines.every((line: unknown) => typeof line === 'string')
    )
      throw new ProbeError(
        'internal',
        'vendor diagnostic selectors did not return text',
      )
    return lines
  }
  const adapter: DiagnosticEngine = {
    previewNode: (node) => Reflect.apply(previewNode, injected, [node]),
    run(hooks, invoke) {
      const overrides = [
        diagnosticOverride(injected, 'previewNode', hooks.previewNode),
        diagnosticOverride(
          injected,
          '_generateSelectors',
          (elements: Element[]) =>
            hooks.generateSelectors(elements, () =>
              generateSelectors(elements),
            ),
        ),
      ]
      const installed: DiagnosticOverride[] = []
      try {
        for (const override of overrides) {
          try {
            Object.defineProperty(
              injected,
              override.property,
              override.replacement,
            )
          } catch {
            throw new ProbeError(
              'internal',
              'vendor diagnostic hooks cannot be installed',
            )
          }
          installed.push(override)
        }
        // Returning a Promise restores now. A producer after an await must enter its own
        // request's scope; leaving hooks installed would change a concurrent call's policy.
        return invoke()
      } finally {
        restoreDiagnostics(injected, installed)
      }
    },
    arm(preview, ...args) {
      // The vendor's setup, listener and stop arrows capture this receiver. Its sole state
      // write must reach the real engine: the constructor's global listener reads that slot,
      // and stop compares its listener identity before clearing it. No shared method lease
      // survives a request, and an intervening opposite-policy call cannot change this arm.
      const receiver: InjectedScript = Object.create(injected, {
        previewNode: { value: preview },
        _hitTargetInterceptor: {
          get: () => Reflect.get(injected, '_hitTargetInterceptor'),
          set(value: unknown) {
            if (
              (value !== undefined && typeof value !== 'function') ||
              !Reflect.set(injected, '_hitTargetInterceptor', value, injected)
            )
              throw new ProbeError(
                'internal',
                'vendor hit-target state could not be forwarded',
              )
          },
        },
      })
      return Reflect.apply(setup, receiver, args)
    },
  }
  diagnosticEngines.set(injected, adapter)
  return adapter
}

/**
 * The aria snapshot a failing poll of `element` reports (upstream's private
 * `InjectedScript._ariaSnapshotForExpect` at v1.64.1): the element's subtree for `toHaveText`
 * and `toContainText`, its own line for the other property matchers, the body when the element
 * or its frame is hidden or the matcher is a page matcher, and none for count or the arrays.
 */
export function expectFailureSnapshot(
  injected: InjectedScript,
  element: Element,
  params: FrameExpectParams,
  frameVisible: boolean,
): string | undefined {
  const snapshot: unknown = Reflect.get(injected, '_ariaSnapshotForExpect')
  if (typeof snapshot !== 'function')
    throw new ProbeError(
      'internal',
      'the vendored InjectedScript has no _ariaSnapshotForExpect',
    )
  const text: unknown = Reflect.apply(snapshot, injected, [
    element,
    params,
    frameVisible,
  ])
  return typeof text === 'string' ? text : undefined
}

/** The `src` of an `<iframe>` or `<frame>`; undefined for any other element. */
export function frameSource(element: Element): string | undefined {
  if (element.nodeName !== 'IFRAME' && element.nodeName !== 'FRAME')
    return undefined
  const src: unknown = Reflect.get(element, 'src')
  return typeof src === 'string' ? src : ''
}
