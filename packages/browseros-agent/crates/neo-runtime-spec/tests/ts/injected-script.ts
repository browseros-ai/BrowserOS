// The smallest stand-in for Playwright's `InjectedScript` that the negative cases need: they
// check the contract's shapes, never the script's methods. neo-runtime-injected checks its real
// glue against the vendored `InjectedScript` itself; this crate keeps no copy of Playwright's
// page code.
// A declarations-only `.ts` module, not a `.d.ts`: packages/browseros-agent/.gitignore ignores
// `**/tests/**/*.d.ts`, so a `.d.ts` here would be missing from a clean checkout.

export declare class InjectedScript {
  previewNode(node: Node): string
}
