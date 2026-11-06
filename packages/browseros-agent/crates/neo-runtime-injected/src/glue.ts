/**
 * The glue table: the page functions Playwright's server sends to the page as source text
 * (`server/frameSelectors.ts`, `frames.ts`, `page.ts`), written once as named TypeScript
 * functions over Playwright's real, unchanged `InjectedScript`. neo-runtime calls each by name
 * through `dist/call.js` with the JSON params the spec declares.
 *
 * `satisfies InjectedApi<InjectedScript>` ties two things in one `tsc` check: every call in
 * neo-runtime-spec's `spec/injected.yml` is here with the generated params and result types,
 * and every body calls `InjectedScript` with Playwright's own signatures.
 *
 * Import rule (Playwright's DEPS.list, enforced by build.ts): this file imports vendored TYPES
 * only; the one value import of vendored code is `install.ts`'s `InjectedScript` constructor.
 *
 * Every function reports the page at one instant. None retries, waits on a selector or owns a
 * deadline: neo-runtime does. Each mirrors Playwright's page-side code in behaviour, with no
 * extra checks, because Neo's results and errors are compared with Playwright's.
 */

import type { InjectedApi } from '@browseros/neo-runtime-spec/injected'
import type { InjectedScript } from '../vendor/playwright/injected/src/injectedScript'

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
    return elements[0]
  },
} satisfies InjectedApi<InjectedScript>
