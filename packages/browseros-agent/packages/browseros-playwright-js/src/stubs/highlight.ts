// Build-time stand-in for upstream highlight.ts: Neo never draws Playwright's inspector
// overlay. InjectedScript only reaches it through its highlight methods, which Neo never
// calls. It sits at the upstream module's name so the type check resolves `./highlight` here
// too (tsconfig.json rootDirs).
export class Highlight {
  // Types the overlay methods InjectedScript calls, for the type check only. None exists at
  // runtime, so a call that ever happened would throw rather than silently draw nothing.
  // biome-ignore lint/suspicious/noExplicitAny: upstream reads these methods' results as its own types.
  readonly [method: string]: (...args: any[]) => any

  // biome-ignore lint/complexity/noUselessConstructor: InjectedScript passes itself; the type check must accept that.
  constructor(_injected: unknown) {}
  install() {}
  uninstall() {}
}
