// Type-only stand-in for upstream packages/injected/src/selectorEngine.ts, which the vendored
// engine imports with `import type` and is therefore not vendored (the bundler erases it).
// The type check resolves `./selectorEngine` here through tsconfig.json rootDirs; the copy
// is upstream's at v1.64.1.
export type SelectorRoot = Element | ShadowRoot | Document

export interface SelectorEngine {
  // biome-ignore lint/suspicious/noExplicitAny: upstream's signature, kept as it is.
  queryAll(root: SelectorRoot, selector: string | any): Element[]
}
