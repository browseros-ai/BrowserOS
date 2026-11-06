// Declarations mirroring the parts of Playwright's injected script that the glue sample uses,
// copied from the commit pinned in spec/playwright/UPSTREAM: packages/injected/src/
// injectedScript.ts (InjectedScriptOptions, InjectedScript), packages/isomorphic/ariaSnapshot.ts
// (AriaRole, AriaNodeJSON) and packages/injected/src/ariaSnapshot.ts (AriaTreeOptions).
// neo-runtime-injected checks its glue against the vendored sources themselves; this crate keeps
// no copy of them, so these signatures change only with a Playwright upgrade.
// A declarations-only `.ts` module, not a `.d.ts`: packages/browseros-agent/.gitignore ignores
// `**/tests/**/*.d.ts`, so a `.d.ts` here would be missing from a clean checkout.

export type AriaRole =
  | 'alert'
  | 'alertdialog'
  | 'application'
  | 'article'
  | 'banner'
  | 'blockquote'
  | 'button'
  | 'caption'
  | 'cell'
  | 'checkbox'
  | 'code'
  | 'columnheader'
  | 'combobox'
  | 'complementary'
  | 'contentinfo'
  | 'definition'
  | 'deletion'
  | 'dialog'
  | 'directory'
  | 'document'
  | 'emphasis'
  | 'feed'
  | 'figure'
  | 'form'
  | 'generic'
  | 'grid'
  | 'gridcell'
  | 'group'
  | 'heading'
  | 'img'
  | 'insertion'
  | 'link'
  | 'list'
  | 'listbox'
  | 'listitem'
  | 'log'
  | 'main'
  | 'mark'
  | 'marquee'
  | 'math'
  | 'meter'
  | 'menu'
  | 'menubar'
  | 'menuitem'
  | 'menuitemcheckbox'
  | 'menuitemradio'
  | 'navigation'
  | 'none'
  | 'note'
  | 'option'
  | 'paragraph'
  | 'presentation'
  | 'progressbar'
  | 'radio'
  | 'radiogroup'
  | 'region'
  | 'row'
  | 'rowgroup'
  | 'rowheader'
  | 'scrollbar'
  | 'search'
  | 'searchbox'
  | 'separator'
  | 'slider'
  | 'spinbutton'
  | 'status'
  | 'strong'
  | 'subscript'
  | 'superscript'
  | 'switch'
  | 'tab'
  | 'table'
  | 'tablist'
  | 'tabpanel'
  | 'term'
  | 'textbox'
  | 'time'
  | 'timer'
  | 'toolbar'
  | 'tooltip'
  | 'tree'
  | 'treegrid'
  | 'treeitem'

export type AriaNodeJSON = {
  role: AriaRole | 'iframe' | 'text'
  name?: string
  checked?: true | 'mixed'
  disabled?: true
  expanded?: true
  active?: true
  invalid?: true | 'grammar' | 'spelling'
  level?: number
  pressed?: true | 'mixed'
  selected?: true
  ariaHidden?: true
  ref?: string
  cursor?: 'pointer'
  box?: { x: number; y: number; width: number; height: number }
  url?: string
  placeholder?: string
  text?: string
  children?: (AriaNodeJSON | string)[]
}

export type AriaSnapshotJSON = AriaNodeJSON[]

export type AriaTreeOptions = {
  mode: 'ai' | 'default' | 'codegen' | 'autoexpect'
  refPrefix?: string
  doNotRenderActive?: boolean
  depth?: number
  boxes?: boolean
}

export type ParsedSelector = {
  parts: { name: string; body: unknown; source: string }[]
  capture?: number
}

export type InjectedScriptOptions = {
  isUnderTest: boolean
  sdkLanguage: 'javascript' | 'python' | 'java' | 'csharp' | 'jsonl'
  frameSeq: number
  testIdAttributeName: string
  stableRafCount: number
  browserName: string
  shouldPrependErrorPrefix?: boolean
  isUtilityWorld?: boolean
  customEngines: { name: string; source: string }[]
}

export declare class InjectedScript {
  constructor(
    window: Window & typeof globalThis,
    options: InjectedScriptOptions,
  )
  parseSelector(selector: string): ParsedSelector
  querySelectorAll(selector: ParsedSelector, root: Node): Element[]
  checkDeprecatedSelectorUsage(
    selector: ParsedSelector,
    matches: Element[],
  ): void
  previewNode(node: Node): string
  strictModeViolationError(selector: ParsedSelector, matches: Element[]): Error
  createStacklessError(message: string): Error
  ariaSnapshotJSON(
    node: Node,
    options: AriaTreeOptions & { depth?: number },
  ): {
    json: AriaSnapshotJSON
    iframeRefs: string[]
    iframeDepths: Record<string, number>
  }
}
