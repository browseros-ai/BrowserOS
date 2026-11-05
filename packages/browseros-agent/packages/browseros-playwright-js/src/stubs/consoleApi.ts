// Build-time stand-in for upstream consoleApi.ts, which would install the window.playwright
// devtools helper. InjectedScript constructs one unconditionally, so the class must exist.
// It sits at the upstream module's name so the type check resolves `./consoleApi` here too
// (tsconfig.json rootDirs).
export class ConsoleAPI {
  // biome-ignore lint/complexity/noUselessConstructor: InjectedScript passes itself; the type check must accept that.
  constructor(_injected: unknown) {}
}
