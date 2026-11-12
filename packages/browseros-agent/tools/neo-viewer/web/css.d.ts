/**
 * Stylesheets imported for their side effect (the fonts in `main.tsx`). Bun's bundler emits them
 * and their font files with the page; TypeScript only needs to know such an import is allowed
 * (TS2882 otherwise rejects a side-effect import with no declaration).
 */
declare module '*.css'
