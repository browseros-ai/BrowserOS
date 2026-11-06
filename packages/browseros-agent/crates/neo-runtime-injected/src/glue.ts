/**
 * The glue table: the page functions Playwright's server sends to the page as source text
 * (`server/frameSelectors.ts`, `frames.ts`, `page.ts`), written once as named TypeScript
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
 * Every function reports the page at one instant. None retries, waits on a selector or owns a
 * deadline: neo-runtime does. Each mirrors Playwright's page-side code in behaviour, with no
 * extra checks, because Neo's results and errors are compared with Playwright's.
 */

import type {
  AriaNode,
  AriaRole,
  InjectedApi,
} from '@browseros/neo-runtime-spec/injected'
import type { InjectedScript } from '../vendor/playwright/injected/src/injectedScript'
import type { AriaNodeJSON } from '../vendor/playwright/isomorphic/ariaSnapshot'

/** The Playwright page types behind the spec's named handle kinds; it names none yet. */
type Handles = Record<never, never>

/**
 * Parses one frame chunk, queries the document, and applies Playwright's strict mode. A strict
 * violation throws Playwright's stackless error, so CDP's `exception.description` reads
 * `Error: strict mode violation: …`, as Playwright reports it.
 */
function query(
  injected: InjectedScript,
  selector: string,
  strict: boolean,
): Element[] {
  const parsed = injected.parseSelector(selector)
  const elements = injected.querySelectorAll(parsed, document)
  injected.checkDeprecatedSelectorUsage(parsed, elements)
  if (strict && elements.length > 1)
    throw injected.strictModeViolationError(parsed, elements)
  return elements
}

export const glue = {
  title: () => ({ value: document.title }),

  textContent: (injected, { selector, strict }) => {
    const [element] = query(injected, selector, strict)
    if (!element) return {}
    return {
      matched: {
        value: element.textContent,
        preview: injected.previewNode(element),
      },
    }
  },

  ariaSnapshot: (injected, { selector, strict, mode, depth, boxes }) => {
    const [element] = query(injected, selector, strict)
    if (!element) return {}
    const { json, iframeRefs, iframeDepths } = injected.ariaSnapshotJSON(
      element,
      { mode, depth, boxes },
    )
    const iframes = iframeRefs
      .filter((ref) => ref in iframeDepths)
      .map((ref) => ({ ref, depth: iframeDepths[ref] }))
    return { snapshot: { nodes: json, iframes } }
  },

  frameOwner: (injected, { selector, strict }) => {
    const elements = query(injected, selector, strict)
    for (const element of elements) {
      if (element.nodeName !== 'IFRAME' && element.nodeName !== 'FRAME')
        throw injected.createStacklessError(
          `Selector "${selector}" resolved to ${injected.previewNode(element)}, <iframe> was expected`,
        )
    }
    return { element: elements[0] }
  },
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
