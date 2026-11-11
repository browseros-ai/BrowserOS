/**
 * What one booted world owns and every handler shares: Playwright's engine for this
 * document, the pin table and Neo's ref map.
 *
 * This is also the one file that reaches into InjectedScript's private fields. Each access
 * checks the shape it expects and fails as `internal` otherwise, so an upgrade that renames
 * a field breaks loudly at the first request instead of resolving refs wrongly.
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
  private readonly elements = new Map<Pin, Element>()
  /** The pins of each entry, oldest first: one for `add`, all of a batch for `addAll`. */
  private readonly entries = new Map<number, Pin[]>()
  private next = 1
  private nextEntry = 1

  add(element: Element): Pin {
    const [pin = 0] = this.addAll([element])
    return pin
  }

  /** Pins every element, in order, as one entry toward the limit. */
  addAll(elements: Element[]): Pin[] {
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
    const element = this.elements.get(pin)
    return element?.isConnected ? element : undefined
  }

  /**
   * The pinned element, in the document or not, until its pin is evicted: what Rust makes a
   * handle of. A match that left the page after it was pinned still gets its handle, which
   * then reads as detached, as upstream's handle to that element would.
   */
  held(pin: Pin): Element | undefined {
    return this.elements.get(pin)
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
