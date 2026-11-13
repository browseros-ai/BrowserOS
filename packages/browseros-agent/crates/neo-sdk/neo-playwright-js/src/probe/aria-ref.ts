/**
 * Neo's `aria-ref` engine and the per-document ref map behind it (design D3, decisions.md).
 *
 * The built-in engine reads only the latest snapshot (`_lastAriaSnapshotForQuery`, which
 * every snapshot replaces, default-mode and scoped ones included) and ignores the query
 * root. Neo's engine:
 * - keeps ref -> element for snapshots and explicit `locator.ref()` reads, so an earlier
 *   ref still names its element after a later or scoped snapshot;
 * - holds elements weakly, so a removed element is collected and its ref fails;
 * - answers only inside the query root, so `filter({ has: page.getByRef('e7') })` keeps
 *   the one row that holds e7 instead of every row;
 * - records refs it cannot resolve, so the op fails at once (`element_detached`) instead of
 *   waiting for a ref that can never come back. It never re-finds an element by role and
 *   name, which could bind the ref to a different element.
 *
 * The engine goes into InjectedScript's table directly, not through `customEngines`: those
 * are evaluated from source text, and a closure over this map cannot travel as source. (The
 * eval itself would work: scripts/probe-smoke.mjs measured eval allowed in this world under
 * a page CSP without 'unsafe-eval'.)
 */
import type { InjectedScript } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import type { QueryEngine } from './world'

/** The engine's own scope test: the composed tree, so refs inside open shadow roots count. */
export type ScopeUtils = Pick<InjectedScript['utils'], 'isInsideScope'>

export class RefMap {
  private readonly byRef = new Map<string, WeakRef<Element>>()
  private printedByElement = new WeakMap<Element, string>()
  private mintedByElement = new WeakMap<Element, string>()
  private nextMinted = 1
  private frameKeys = new WeakMap<Element, string>()
  private nextFrameKey = 1
  private unresolved: string[] = []

  /** Replaces the built-in `aria-ref` engine in an engine table (world.ts `engineTable`). */
  install(engines: Map<string, QueryEngine>, utils: ScopeUtils): void {
    engines.set('aria-ref', {
      queryAll: (root, body) => {
        const ref = String(body)
        const element = this.byRef.get(ref)?.deref()
        if (!element?.isConnected) {
          this.unresolved.push(ref)
          return []
        }
        return utils.isInsideScope(root, element) ? [element] : []
      },
    })
  }

  /**
   * Runs one query and reports the first ref it could not resolve. Queries are synchronous
   * and the page has one thread, so nested parts (`internal:has`) report into the same run.
   */
  query(run: () => Element[]): {
    elements: Element[]
    staleRef: string | undefined
  } {
    this.unresolved = []
    const elements = run()
    const [staleRef] = this.unresolved
    this.unresolved = []
    return { elements, staleRef }
  }

  /**
   * Adds the refs an ai-mode snapshot printed (the engine's `_lastAriaSnapshotForQuery.info`)
   * and drops collected elements. The snapshot handler must call it after every ai-mode
   * snapshot, because `forget` finds the expandos to delete through this map.
   */
  remember(refs: Map<string, { element: Element }>): void {
    for (const [ref, { element }] of refs) {
      this.byRef.set(ref, new WeakRef(element))
      // The engine may mint a new ref after a role or name change. Prefer that current ref,
      // while retaining every earlier alias for callers that already checked the node.
      this.printedByElement.set(element, ref)
    }
    for (const [ref, weak] of this.byRef)
      if (!weak.deref()) this.byRef.delete(ref)
  }

  /**
   * Pins identity for a host's later run without adding a target to the snapshot. Minted
   * refs share the document's weak map but never set Playwright's `_ariaRef` expando, so a
   * later snapshot still decides independently whether the element gets an engine ref.
   */
  forElement(element: Element, seq: number): string {
    const printed = this.printedByElement.get(element)
    if (printed !== undefined && this.byRef.get(printed)?.deref() === element)
      return printed
    const known = this.mintedByElement.get(element)
    if (known !== undefined) return known
    const ref = `${seq === 0 ? '' : `f${seq}`}n${this.nextMinted++}`
    this.mintedByElement.set(element, ref)
    this.byRef.set(ref, new WeakRef(element))
    return ref
  }

  /**
   * A Neo-internal key (`#4`) for an iframe a resolver hop went through, so a later ref jump
   * into that frame finds the same iframe again for hit checks and coordinates. Snapshot refs
   * never start with `#`, so the two cannot collide.
   */
  frameKey(iframe: Element): string {
    const known = this.frameKeys.get(iframe)
    if (known !== undefined) return known
    const key = `#${this.nextFrameKey++}`
    this.frameKeys.set(iframe, key)
    this.byRef.set(key, new WeakRef(iframe))
    return key
  }

  /**
   * A re-key: forgets every ref and frame key, and deletes the `_ariaRef` expando the engine
   * keeps on each element it minted a ref for. Without that, the next snapshot would print
   * an element's old ref, minted under another seq, instead of a fresh one.
   */
  forget(): void {
    for (const weak of this.byRef.values()) {
      const element = weak.deref()
      if (element) Reflect.deleteProperty(element, '_ariaRef')
    }
    this.byRef.clear()
    this.printedByElement = new WeakMap()
    this.mintedByElement = new WeakMap()
    this.frameKeys = new WeakMap()
  }
}
