/**
 * The glue table: the page functions Playwright's server sends to the page as source text
 * (`server/frameSelectors.ts`, `frames.ts`, `page.ts`, `dom.ts`), written once as named TypeScript
 * functions over Playwright's real, unchanged `InjectedScript`. neo-runtime calls each by name
 * through `dist/call.js`, with the JSON params and then the handles the spec declares
 * (`neo_runtime_spec::InjectedCall` documents the convention).
 *
 * `satisfies InjectedApi<InjectedScript, Handles>` ties three things in one `tsc` check: every
 * call in neo-runtime-spec's `spec/injected.yml` is here with the generated params, handle and
 * result types; every body calls `InjectedScript` with Playwright's own signatures; and each
 * named handle kind is bound to the one Playwright page type that the call producing it and the
 * call consuming it both use.
 *
 * Import rule (Playwright's DEPS.list, enforced by build.ts): this file imports vendored TYPES
 * only; the one value import of vendored code is `install.ts`'s `InjectedScript` constructor.
 *
 * Every function reports the page at one instant, except `checkElementStates`, which awaits about
 * two animation frames for `stable`. None retries, waits on a selector or owns a deadline:
 * neo-runtime does. Each mirrors Playwright's page-side code in behaviour, with no extra checks,
 * so a call's results and errors are the ones Playwright's page code gives.
 */

import type {
  AriaNode,
  AriaRole,
  InjectedApi,
} from '@browseros/neo-runtime-spec/injected'
import type {
  HitTargetInterceptionResult,
  InjectedScript,
} from '../vendor/playwright/injected/src/injectedScript'
import type { AriaNodeJSON } from '../vendor/playwright/isomorphic/ariaSnapshot'

/** An element with the aria ref Playwright's snapshots keep on it (`ariaSnapshot.ts`
 * `computeAriaRef`: `(element as any)._ariaRef = ariaRef;`). */
type NamedElement = Element & { _ariaRef?: { ref: string } }

/**
 * Neo: every owner `frameOwnerRef` named, by its ref, kept weakly. `frameOwnerByRef` finds an
 * owner here first: CDP hands `frameOwnerRef` owners a selector cannot reach (inside a closed
 * shadow root), and a ref Neo answers must route. One table per document, as the glue is.
 */
const namedOwners = new Map<string, WeakRef<Element>>()

/** The Playwright page types behind the spec's named handle kinds. */
type Handles = {
  HitTargetInterceptor: HitTargetInterceptionResult
  ElementList: Record<string, Element>
}

/**
 * Parses one frame chunk, queries the document, and applies Playwright's checks in Playwright's
 * order (`frameSelectors.ts` `_callOnSelectorInternal`): mark the matches the call asks for,
 * reject a deprecated selector, then strict mode. A strict violation throws Playwright's
 * stackless error, so CDP's `exception.description` reads `Error: strict mode violation: …`, as
 * Playwright reports it.
 *
 * `markTargets` is Playwright's: `all` for an action (`resolve`), `first` for a read of the first
 * match (`textContent`, which resets the marks when nothing matches), `none` otherwise.
 */
function query(
  injected: InjectedScript,
  selector: string,
  strict: boolean,
  {
    markTargets = 'none',
    root = document,
  }: { markTargets?: 'all' | 'first' | 'none'; root?: Node } = {},
): Element[] {
  const parsed = injected.parseSelector(selector)
  const elements = injected.querySelectorAll(parsed, root)
  // Before the checks, as in Playwright: the page's own listeners see the matches marked even
  // when strict mode then fails the call.
  if (markTargets === 'all') injected.markTargetElements(new Set(elements))
  else if (markTargets === 'first' && elements.length)
    injected.markTargetElements(new Set([elements[0]]))
  else if (markTargets === 'first') injected.markTargetElements(new Set())
  injected.checkDeprecatedSelectorUsage(parsed, elements)
  if (strict && elements.length > 1)
    throw injected.strictModeViolationError(parsed, elements)
  return elements
}

/** The listeners `setupDragListeners` armed in this document, until `cleanupDrag` reads them.
 * Playwright keeps it as `window.__cleanupDrag`; the glue's own scope keeps it from the page. */
let cleanupDrag: (() => Promise<boolean>) | undefined

/** Neo: elements as a null-prototype object's own properties `0`, `1`, …, which CDP reports
 * as a plain object (an array would carry the subtype `array`), so Rust lists them with
 * `Runtime.getProperties` as handles of the matches. */
function elementList(elements: Element[]): Record<string, Element> {
  return Object.assign(Object.create(null), elements)
}

export const glue = {
  title: () => ({ value: document.title }),

  /** `frames.ts:855-864`: one waitForSelector probe, without marking the matches. */
  selectorState: (injected, { selector, strict }) => {
    const elements = query(injected, selector, strict)
    const [element] = elements
    if (!element) return {}
    return {
      matched: {
        count: elements.length,
        visible: injected.elementState(element, 'visible', true).matches,
        preview: injected.previewNode(element),
      },
    }
  },

  /** `selectorState` plus the first match as a handle, for a wait that returns its element. */
  // Playwright: `server/frames.ts` `Frame.waitForSelector`: `return { log, element, visible, attached: !!element };`
  selectorElement: (injected, { selector, strict }) => {
    const elements = query(injected, selector, strict)
    const [element] = elements
    if (!element) return { count: 0, visible: false }
    return {
      element,
      count: elements.length,
      visible: injected.elementState(element, 'visible', true).matches,
      preview: injected.previewNode(element),
    }
  },

  /** `selectorElement` within an adopted element handle, for `ElementHandle.$` and `$eval`. */
  // Playwright: `server/frameSelectors.ts` `FrameSelectors._callOnSelectorInternal`: `const elements = injected.querySelectorAll(params.info.parsed, params.scope || document);`
  scopedSelectorElement: (injected, { selector, strict }, root) => {
    const elements = query(injected, selector, strict, { root })
    const [element] = elements
    if (!element) return { count: 0, visible: false }
    return {
      element,
      count: elements.length,
      visible: injected.elementState(element, 'visible', true).matches,
      preview: injected.previewNode(element),
    }
  },

  /** Every match, as Playwright's `queryAll` resolves it: the elements, by index. */
  // Playwright: `server/frameSelectors.ts` `FrameSelectors.queryAll`: `const resolved = await this.callOnSelectorHandle({ selector, strict: false, scope }, ({ elements }) => elements, {});`
  queryAll: (injected, { selector }) => ({
    elements: elementList(query(injected, selector, false)),
  }),

  // Playwright: `server/frameSelectors.ts` `FrameSelectors.queryAll`: `const resolved = await this.callOnSelectorHandle({ selector, strict: false, scope }, ({ elements }) => elements, {});`
  scopedQueryAll: (injected, { selector }, root) => ({
    elements: elementList(query(injected, selector, false, { root })),
  }),

  // Playwright: `server/dom.ts` `ElementHandle._initializePreview`: `this._setPreview(await utility.evaluate((injected, e) => 'JSHandle@' + injected.previewNode(e), this));`
  listPreviews: (injected, _params, list) => ({
    previews: Object.values(list).map((element) =>
      injected.previewNode(element),
    ),
  }),

  // Neo: the anchor `main_world.rs` resolves into the page's own world; Playwright's server
  // finds that world by its execution context, which Neo does not watch.
  // Rust only reads the node's `backendNodeId` (`DOM.describeNode`), which a `Document` has like
  // any node; the cast is what lets the spec's one node handle kind carry it.
  document: () => ({ node: document as unknown as Element }),

  // Playwright: `server/dom.ts` `ElementHandle._initializePreview`: `this._setPreview(await utility.evaluate((injected, e) => 'JSHandle@' + injected.previewNode(e), this));`
  previewNode: (injected, _params, node) => ({
    preview: injected.previewNode(node),
  }),

  textContent: (injected, { selector, strict }) => {
    const [element] = query(injected, selector, strict, {
      markTargets: 'first',
    })
    if (!element) return {}
    return {
      matched: {
        value: element.textContent,
        preview: injected.previewNode(element),
      },
    }
  },

  innerText: (injected, { selector, strict }) => {
    const [element] = query(injected, selector, strict, {
      markTargets: 'first',
    })
    if (!element) return {}
    if (element.namespaceURI !== 'http://www.w3.org/1999/xhtml')
      throw injected.createStacklessError('Node is not an HTMLElement')
    return {
      matched: {
        value: (element as HTMLElement).innerText,
        preview: injected.previewNode(element),
      },
    }
  },

  getAttribute: (injected, { selector, strict, name }) => {
    const [element] = query(injected, selector, strict, {
      markTargets: 'first',
    })
    if (!element) return {}
    const value = element.getAttribute(name)
    return {
      matched: {
        ...(value === null ? {} : { value }),
        preview: injected.previewNode(element),
      },
    }
  },

  isVisible: (injected, { selector, strict, frameVisible }) => {
    const [element] = query(injected, selector, strict)
    return {
      value:
        !!element &&
        frameVisible &&
        injected.elementState(element, 'visible', true).matches,
    }
  },

  /**
   * Every `Frame.is*` read that waits for its match: one state of the first match, labels
   * followed. `elementState` throws for a state the element cannot have.
   *
   * Playwright: `server/frames.ts` `Frame._elementState`: `return { result: injected.elementState(element, data.state) };`
   */
  elementStateOnSelector: (injected, { selector, strict, state }) => {
    const [element] = query(injected, selector, strict, {
      markTargets: 'first',
    })
    if (!element) return {}
    const { matches, received } = injected.elementState(element, state)
    return {
      matched: {
        matches,
        received,
        preview: injected.previewNode(element),
      },
    }
  },

  /** Playwright: `server/frames.ts` `Frame.innerHTML`: `(injected, element) => ({ result: element.innerHTML })` */
  innerHTML: (injected, { selector, strict }) => {
    const [element] = query(injected, selector, strict, {
      markTargets: 'first',
    })
    if (!element) return {}
    return {
      matched: {
        value: element.innerHTML,
        preview: injected.previewNode(element),
      },
    }
  },

  /**
   * Playwright: `server/frames.ts` `Frame.ariaSnapshotJSON`: `injected.ariaSnapshotJSON(element, opts).json`
   * Playwright: `server/page.ts` `ariaSnapshotJSONForFrame`: `return injected.ariaSnapshotJSON(elements[0], ariaOptions);`
   */
  ariaSnapshot: (injected, { selector, strict, mode, depth, boxes }) => {
    // Playwright: `server/frames.ts` `Frame._retryWithProgressIfNotConnected`: `markTargets: 'all'`
    const [element] = query(injected, selector, strict, {
      markTargets: strict && mode === 'default' ? 'all' : 'none',
    })
    if (!element) return {}
    const { json, iframeRefs, iframeDepths } = injected.ariaSnapshotJSON(
      element,
      { mode, depth, boxes },
    )
    const iframes = iframeRefs
      .filter((ref) => ref in iframeDepths)
      .map((ref) => ({ ref, depth: iframeDepths[ref] }))
    // Neo: return the preview with the tree so the wait loop logs the same matched element.
    return {
      snapshot: { nodes: json, iframes },
      preview: injected.previewNode(element),
    }
  },

  frameOwner: (injected, { selector, strict }) => {
    const elements = query(injected, selector, strict)
    for (const element of elements) {
      if (element.nodeName !== 'IFRAME' && element.nodeName !== 'FRAME')
        throw injected.createStacklessError(
          `Selector "${selector}" resolved to ${injected.previewNode(element)}, <iframe> was expected`,
        )
    }
    const element = elements[0]
    return {
      element,
      visible: !!element && injected.utils.isElementVisible(element),
    }
  },

  /** Neo: one hop of a main-rooted path ref, by the owner's kept aria ref rather than the
   * document's last snapshot, so a later snapshot of another part of the document leaves it. */
  frameOwnerByRef: (injected, { ref }) => {
    const named = namedOwners.get(ref)?.deref()
    // Only an owner still connected in this document: one moved into another document (a
    // sibling frame's) carries its ref along, but this document's path no longer leads to it.
    const element =
      named?.isConnected &&
      named.ownerDocument === document &&
      (named as NamedElement)._ariaRef?.ref === ref
        ? named
        : injected
            .querySelectorAll(injected.parseSelector('iframe, frame'), document)
            .find((owner) => (owner as NamedElement)._ariaRef?.ref === ref)
    return {
      element,
      visible: !!element && injected.utils.isElementVisible(element),
    }
  },

  /** Neo: the owner's aria ref, from a snapshot of the owner alone through the one
   * `InjectedScript` snapshot that leaves the document's last snapshot (and so its `aria-ref`
   * selectors) as it was. */
  frameOwnerRef: (injected, _params, element) => {
    injected.ariaSnapshotForExpectFailure(element, { mode: 'ai' })
    const ref = (element as NamedElement)._ariaRef?.ref
    if (ref === undefined) return {}
    namedOwners.set(ref, new WeakRef(element))
    return { ref }
  },

  /** Playwright: `server/frames.ts` `Frame._content`: `return await injected.evaluate((injected, includeShadow) => injected.documentContent(includeShadow), !!options.includeShadow);` */
  documentContent: (injected, { includeShadow }) => ({
    value: injected.documentContent(includeShadow),
  }),

  /** Playwright: `server/frames.ts` `Frame.setContent`: `document.write(html);` */
  writeContent: (_injected, { html }) => {
    // No console tag marks the open: neo-runtime waits for the reopened document's lifecycle
    // events, from its `init`, on the frame's own session.
    document.open()
    document.write(html)
    document.close()
    return {}
  },

  /** Playwright: `server/frames.ts` `Frame.resolveSelector`: `return injected.generateSelectorSimple(node as unknown as Element);` */
  generateSelector: (injected, { selector }) => {
    const [element] = query(injected, selector, false)
    if (!element) return {}
    return { generated: injected.generateSelectorSimple(element) }
  },

  /** Playwright: `server/frames.ts` `Frame.resolveSelector`: `return injected.generateSelectorSimple(node as unknown as Element);` */
  generateOwnerSelector: (injected, _params, element) => ({
    generated: injected.generateSelectorSimple(element),
  }),

  describeSelector: (injected, { selector }) => {
    const locator = injected.utils.asLocator('javascript', selector)
    try {
      injected.parseSelector(selector)
      return { locator }
    } catch (error) {
      return { locator, invalidSelector: error.message }
    }
  },

  /** The page callback of `frames.ts` `_retryWithProgressIfNotConnected`, minus its log text. */
  resolve: (injected, { selector, strict }) => {
    const elements = query(injected, selector, strict, { markTargets: 'all' })
    const [element] = elements
    return {
      element,
      count: elements.length,
      preview: element ? injected.previewNode(element) : undefined,
    }
  },

  checkElementStates: async (injected, { states, frameVisible }, element) => {
    const outcome = await injected.checkElementStates(
      element,
      states,
      frameVisible,
    )
    return outcome === undefined ? {} : { outcome }
  },

  /** `dom.ts` `doScrollIntoView`'s page function, for loop B's forced scroll options. */
  scrollIntoView: (_injected, { block, inline }, element) => {
    if (element.nodeType === Node.ELEMENT_NODE)
      element.scrollIntoView({ block, inline, behavior: 'instant' })
    return {}
  },

  // Playwright: `server/dom.ts` `ElementHandle._clickablePoint`: `({ width: innerWidth, height: innerHeight })`
  viewportSize: () => ({ width: innerWidth, height: innerHeight }),

  getElementBorderWidth: (injected, _params, element) =>
    injected.getElementBorderWidth(element),

  describeIFrameStyle: (injected, _params, element) => ({
    style: injected.describeIFrameStyle(element),
  }),

  expectHitTarget: (injected, { point }, element) => ({
    outcome: injected.expectHitTarget(point, element),
  }),

  /** Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `injected.setupHitTargetInterceptor(node, actionType, hitPoint, trial)` */
  setupHitTargetInterceptor: (
    injected,
    { action, hitPoint, blockAllEvents },
    element,
  ) => ({
    outcome: injected.setupHitTargetInterceptor(
      element,
      action,
      hitPoint,
      blockAllEvents,
    ),
  }),

  // Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `window.addEventListener('mousemove', mouseListener, { once: true, capture: true });`
  setupDragListeners: () => {
    let didStartDrag = Promise.resolve(false)
    let dragEvent: Event | null = null
    const dragListener = (event: Event) => {
      dragEvent = event
    }
    const mouseListener = () => {
      didStartDrag = new Promise<boolean>((callback) => {
        window.addEventListener('dragstart', dragListener, {
          once: true,
          capture: true,
        })
        setTimeout(
          () => callback(dragEvent ? !dragEvent.defaultPrevented : false),
          0,
        )
      })
    }
    window.addEventListener('mousemove', mouseListener, {
      once: true,
      capture: true,
    })
    cleanupDrag = async () => {
      const started = await didStartDrag
      window.removeEventListener('mousemove', mouseListener, { capture: true })
      window.removeEventListener('dragstart', dragListener, { capture: true })
      cleanupDrag = undefined
      return started
    }
    return { armed: true }
  },

  // Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `const val = await didStartDrag;`
  cleanupDrag: async () => ({
    started: cleanupDrag ? await cleanupDrag() : false,
  }),

  stopHitTargetInterceptor: (_injected, _params, interceptor) => ({
    outcome: interceptor.stop(),
  }),

  /** `dom.ts` `_fill`'s single page evaluation, without its server-side retry loop. */
  fill: async (injected, { value, force, frameVisible }, element) => {
    if (!force) {
      const outcome = await injected.checkElementStates(
        element,
        ['visible', 'enabled', 'editable', 'non-inert'],
        frameVisible,
      )
      if (outcome) return { outcome }
    }
    return { outcome: injected.fill(element, value) }
  },

  /** `dom.ts` `_selectOption`'s single page evaluation. */
  selectOptions: async (
    injected,
    { options, force, frameVisible },
    element,
  ) => {
    if (!force) {
      const outcome = await injected.checkElementStates(
        element,
        ['visible', 'enabled', 'non-inert'],
        frameVisible,
      )
      if (outcome) return { outcome }
    }
    const outcome = injected.selectOptions(element, options)
    return Array.isArray(outcome) ? { values: outcome } : { outcome }
  },

  elementState: (injected, { state }, element) =>
    injected.elementState(element, state),

  focusElement: (injected, { resetSelectionIfNotFocused }, element) => ({
    outcome: injected.focusNode(element, resetSelectionIfNotFocused),
  }),

  /** Playwright: `server/dom.ts` `ElementHandle._blur`: `injected.blurNode(node)` */
  blurElement: (injected, _params, element) => ({
    outcome: injected.blurNode(element),
  }),

  /** Playwright: `server/dom.ts` `ElementHandle.selectText`: `return injected.selectText(node);` */
  selectText: async (injected, { force, frameVisible }, element) => {
    if (!force) {
      const outcome = await injected.checkElementStates(
        element,
        ['visible'],
        frameVisible,
      )
      if (outcome) return { outcome }
    }
    return { outcome: injected.selectText(element) }
  },

  /** Playwright: `server/frames.ts` `Frame.dispatchEvent`: `injectedScript.dispatchEvent(element, data.type, data.eventInit);` */
  dispatchEvent: (injected, { selector, strict, type, eventInit }) => {
    const [element] = query(injected, selector, strict, {
      markTargets: 'first',
    })
    if (!element) return {}
    injected.dispatchEvent(element, type, JSON.parse(eventInit))
    return { preview: injected.previewNode(element) }
  },

  /** Playwright: `server/highlightController.ts` `HighlightController._resolve`: `await injectedScript.evaluate((injected, highlights) => injected.setHighlights(highlights), highlights);` */
  setHighlights: (injected, { highlights }) => {
    injected.setHighlights(
      highlights.map(({ selector, cssStyle }) => ({
        selector: injected.parseSelector(selector),
        cssStyle,
      })),
    )
    return {}
  },

  /** Playwright: `server/highlightController.ts` `HighlightController.hideHighlights`: `await injectedScript?.evaluate(injected => injected.hideHighlight());` */
  hideHighlight: (injected) => {
    injected.hideHighlight()
    return {}
  },

  inputValue: (injected, { selector, strict }) => {
    const [node] = query(injected, selector, strict, { markTargets: 'first' })
    if (!node) return {}
    const element = injected.retarget(node, 'follow-label')
    if (
      !element ||
      (element.nodeName !== 'INPUT' &&
        element.nodeName !== 'TEXTAREA' &&
        element.nodeName !== 'SELECT')
    )
      throw injected.createStacklessError(
        'Node is not an <input>, <textarea> or <select> element',
      )
    return {
      matched: {
        value: (
          element as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
        ).value,
        preview: injected.previewNode(node),
      },
    }
  },
  queryCount: (injected, { selector }) => ({
    value: query(injected, selector, false).length,
  }),
} satisfies InjectedApi<InjectedScript, Handles>

// Playwright's AriaNodeJSON reaches Rust unchanged, but TypeScript lets a structurally wider
// value through. Pinning the key sets makes a Playwright upgrade that adds a field fail here,
// rather than in Rust's decode at run time. Each `satisfies` fails `tsc` when its type is
// `false`, and declares nothing for the bundle to export.
type SameKeys<A, B> = [keyof A] extends [keyof B]
  ? [keyof B] extends [keyof A]
    ? true
    : false
  : false
true satisfies SameKeys<AriaNodeJSON, AriaNode>
true satisfies AriaNodeJSON['role'] extends AriaRole ? true : false
