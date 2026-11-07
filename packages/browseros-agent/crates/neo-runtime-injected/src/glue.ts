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

/** The Playwright page types behind the spec's named handle kinds. */
type Handles = { HitTargetInterceptor: HitTargetInterceptionResult }

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
  { markTargets = 'none' }: { markTargets?: 'all' | 'first' | 'none' } = {},
): Element[] {
  const parsed = injected.parseSelector(selector)
  const elements = injected.querySelectorAll(parsed, document)
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

  setupHitTargetInterceptor: (
    injected,
    { hitPoint, blockAllEvents },
    element,
  ) => ({
    outcome: injected.setupHitTargetInterceptor(
      element,
      'mouse',
      hitPoint,
      blockAllEvents,
    ),
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

  /**
   * Deviation: `client/locator.ts` `Locator.allTextContents`: `ee => ee.map(e => e.textContent || '')` runs in Neo's isolated world, where Playwright's `$$eval` runs in the page's main world, so a page's own `textContent` override (a prototype patch, a custom element class or a per-instance property) is not seen.
   */
  allTextContents: (injected, { selector }) => ({
    values: query(injected, selector, false).map((e) => e.textContent || ''),
  }),

  /**
   * Deviation: `client/locator.ts` `Locator.allInnerTexts`: `ee => ee.map(e => (e as HTMLElement).innerText)` runs in Neo's isolated world, where Playwright's `$$eval` runs in the page's main world, so a page's own `innerText` override (a prototype patch or a per-instance property) is not seen.
   */
  allInnerTexts: (injected, { selector }) => ({
    values: query(injected, selector, false).map(
      (e) => (e as HTMLElement).innerText,
    ),
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
