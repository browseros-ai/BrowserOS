// Build-time stand-in for upstream ariaSnapshot.ts. Swapped in at switch-over so unmodified
// InjectedScript drops ariaSnapshotDistiller while ARIA snapshots and templates run in Rust.
import type {
  AriaNode,
  AriaSnapshotJSON,
  AriaTemplateNode,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'

export type AriaSnapshot = {
  root: AriaNode
  info: Map<string, { element: Element; nameFromContentRefs: string[] }>
  refs: Map<Element, string>
  iframeRefs: string[]
}

export type AriaTreeOptions = {
  mode: 'ai' | 'default' | 'codegen' | 'autoexpect'
  refPrefix?: string
  doNotRenderActive?: boolean
  depth?: number
  boxes?: boolean
}

export type MatcherReceived = {
  raw: string
  regex: string
}

function unsupportedInPage(name: string): never {
  throw new Error(
    `${name}: ARIA snapshots and template matching run in Rust, not in the page`,
  )
}

export function generateAriaTree(
  _rootElement: Element,
  _publicOptions: AriaTreeOptions,
): AriaSnapshot {
  return unsupportedInPage('generateAriaTree')
}

export function matchesExpectAriaTemplate(
  _rootElement: Element,
  _template: AriaTemplateNode,
): { matches: AriaNode[]; received: MatcherReceived } {
  return unsupportedInPage('matchesExpectAriaTemplate')
}

export function getAllElementsMatchingExpectAriaTemplate(
  _rootElement: Element,
  _template: AriaTemplateNode,
): Element[] {
  return unsupportedInPage('getAllElementsMatchingExpectAriaTemplate')
}

export function renderAriaTreeAsJSON(
  _ariaSnapshot: AriaSnapshot,
  _publicOptions: AriaTreeOptions,
): { json: AriaSnapshotJSON; iframeDepths: Record<string, number> } {
  return unsupportedInPage('renderAriaTreeAsJSON')
}

export function findNewElement(
  _from: AriaNode | undefined,
  _to: AriaNode,
): Element | undefined {
  return unsupportedInPage('findNewElement')
}
