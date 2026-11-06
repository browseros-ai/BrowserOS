// A sample of neo-runtime-injected's glue table, checked against the generated contract and
// Playwright's InjectedScript signatures. Every function must satisfy InjectedApi, so a spec
// change the page code does not follow fails tsc. The bodies port the page functions Playwright's
// server sends as source text (server/frameSelectors.ts, server/page.ts, server/frames.ts).

import type {
  AriaNode,
  AriaRole,
  InjectedApi,
  InjectedInitializer,
} from '../../ts/injected'
import { SPEC_SHA256 } from '../../ts/spec-hash'
import {
  type AriaNodeJSON,
  InjectedScript,
  type InjectedScriptOptions,
} from './injected-script'

// One frame chunk, parsed and queried in the page, with Playwright's strict mode.
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

// The bundle's entry point. neo-runtime evaluates the bundle once per world and document,
// compares `specSha256` with the Rust crate's SPEC_SHA256, then calls `create`.
export const bundle = {
  specSha256: SPEC_SHA256,
  create(initializer: InjectedInitializer) {
    // Every initializer field must be one of Playwright's options, under the same name.
    const fromSpec: Pick<InjectedScriptOptions, keyof InjectedInitializer> =
      initializer
    const options: InjectedScriptOptions = {
      ...fromSpec,
      isUnderTest: false,
      sdkLanguage: 'javascript',
      browserName: 'chromium',
      customEngines: [],
    }
    const injected = new InjectedScript(window, options)
    return Object.fromEntries(
      Object.entries(glue).map(([name, call]) => [
        name,
        (params: never) => call(injected, params),
      ]),
    )
  },
}

// Playwright's AriaNodeJSON reaches Rust unchanged, but TypeScript lets a structurally wider
// value through. Pinning the key sets makes a Playwright upgrade that adds a field fail here,
// rather than in Rust's decode at run time.
export type SameKeys<A, B> = [keyof A] extends [keyof B]
  ? [keyof B] extends [keyof A]
    ? true
    : false
  : false
export type Assert<T extends true> = T
export type AriaNodeMatchesPlaywright = Assert<SameKeys<AriaNodeJSON, AriaNode>>
export type AriaRolesCoverPlaywright = Assert<
  AriaNodeJSON['role'] extends AriaRole ? true : false
>
