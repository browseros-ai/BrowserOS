/**
 * Ambient declarations for the bare `@fontsource-variable/*` side-effect
 * imports in the cockpit (see entrypoints/newtab/main.tsx). These packages
 * ship CSS only and carry no type declarations; their `exports["."]` maps to
 * the variable-font stylesheet, so they are imported by package name rather
 * than by a `.css` path (the weighted `@fontsource/*` imports use `.css`
 * paths and are covered by the bundler's `*.css` module type).
 *
 * The native TypeScript 7 compiler rejects a side-effect import of a module
 * with no type declaration (TS2882), unlike classic tsc which accepted any
 * resolvable package. Declaring them as untyped side-effect modules satisfies
 * the checker without changing what the bundler ships.
 */
declare module '@fontsource-variable/schibsted-grotesk'
declare module '@fontsource-variable/jetbrains-mono'
