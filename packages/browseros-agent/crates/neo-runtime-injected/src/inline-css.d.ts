// Playwright's highlight.ts imports its stylesheet as text (`./highlight.css?inline`), a module
// Playwright types through vite/client. build.ts inlines the file's text the same way.
declare module '*.css?inline' {
  const css: string
  export default css
}
